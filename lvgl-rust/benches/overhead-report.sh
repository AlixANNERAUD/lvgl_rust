#!/usr/bin/env bash
# Writes documentation/reference/overhead.md: run by `mise run overhead`,
# which sets the safe crate's configuration and target directory.
#
# 1. The host benchmark (benches/overhead.rs, release profile) times safe
#    calls against the raw calls doing the same LVGL work.
# 2. The size gate (size-gate/gate.sh, through `mise run size-gate`)
#    links firmware for thumbv6m and riscv32imc and reports the crate's
#    .bss, .data and code next to the whole firmware. It needs the C
#    toolchains of `mise bootstrap packages apply`; without them, the
#    crate's numbers come from the size gate job of the CI run of the last
#    commit that changed the firmware's inputs, when `gh` can read it.
set -euo pipefail
# Untranslated tool output, and decimal points.
export LC_ALL=C

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
out=$root/documentation/reference/overhead.md
work=$root/${CARGO_TARGET_DIR:-target}/overhead-report
rm -rf "$work"
mkdir -p "$work" "$(dirname "$out")"

# --- Measurements ------------------------------------------------------------

LVGL_RUST_OVERHEAD_OUTPUT=$work cargo bench -q -p lvgl-rust --bench overhead --features heap

sizes=$work/sizes.tsv
size_source=
if command -v arm-none-eabi-gcc >/dev/null && command -v riscv64-unknown-elf-gcc >/dev/null; then
    SIZE_GATE_REPORT=$sizes mise run size-gate
    size_source="built on this machine by \`mise run size-gate\`"
else
    echo "overhead: no thumbv6m or riscv32imc C toolchain (mise bootstrap packages apply): reading the size gate of CI" >&2
    inputs=(Cargo.toml Cargo.lock lvgl-rust/build.rs lvgl-rust/conf lvgl-rust/src
        lvgl-rust/size-gate/Cargo.toml lvgl-rust/size-gate/build.rs lvgl-rust/size-gate/src lvgl-rust-gen
        lvgl-rust-sys)
    if git diff --quiet HEAD -- "${inputs[@]}" && command -v gh >/dev/null; then
        commit=$(git log -1 --format=%H -- "${inputs[@]}")
        run=$(gh run list --commit "$commit" --workflow ci.yml --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)
        job=
        if [ -n "$run" ]; then
            job=$(gh run view "$run" --json jobs \
                --jq '.jobs[] | select(.conclusion == "success" and (.name | contains("thumbv6m and riscv32imc"))) | .databaseId' \
                2>/dev/null || true)
        fi
        if [ -n "$job" ]; then
            gh run view --job "$job" --log 2>/dev/null | grep -o 'size gate (.*' >"$work/ci.txt" || true
            # "size gate (<target>, N = <slots>[, <features>]): .bss B B, .data D B, code C B (budget X B)"
            sed -nE 's/^size gate \(([^,]+), N = ([0-9]+)(, ([^)]*))?\): \.bss ([0-9]+) B, \.data ([0-9]+) B, code ([0-9]+) B \(budget ([0-9]+) B\).*/\1\t\2\t\4\t\5\t\6\t\7\t\8/p' \
                "$work/ci.txt" | while IFS=$'\t' read -r target slots features bss data code budget; do
                label="$target, N = $slots${features:+, $features}"
                features=${features:--}
                firmware=$(grep -F "size gate ($label): firmware" "$work/ci.txt" | head -1 |
                    sed -nE 's/.*firmware text ([0-9]+) B, data ([0-9]+) B, bss ([0-9]+) B.*/\1\t\2\t\3/p')
                [ -n "$firmware" ] || firmware=$'-\t-\t-'
                if [ "$slots" = 0 ]; then reg=8; else reg=$((8 * slots + 24)); fi
                if [[ ",$features," == *,debug-counters,* ]]; then counters=8; else counters=0; fi
                printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$target" "$slots" "$features" "$bss" "$data" \
                    "$code" "$budget" "$firmware" "$reg" 36 "$counters"
            done >"$sizes"
            if [ -s "$sizes" ]; then size_source="read from the size gate job of [CI run $run](https://github.com/AlixANNERAUD/lvgl_rust/actions/runs/$run) (commit \`${commit:0:9}\`, the last one that changed the firmware's inputs), since this machine has no thumbv6m or riscv32imc C toolchain"; fi
        fi
    fi
fi

# --- Context -----------------------------------------------------------------

cpu=$(lscpu 2>/dev/null | sed -nE 's/^Model name:[[:space:]]+//p' | grep -v '^-$' | sort -u | paste -sd '/' - || true)
[ -n "$cpu" ] || cpu=$(sed -nE 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -1)
if [ -z "$cpu" ]; then
    vendor=$(lscpu 2>/dev/null | sed -nE 's/^Vendor ID:[[:space:]]+//p' | head -1)
    cpu="${vendor:-unknown} CPU (model not reported)"
fi
virt=$(systemd-detect-virt 2>/dev/null || true)
[ -n "$virt" ] || virt=$(lscpu 2>/dev/null | sed -nE 's/^Hypervisor vendor:[[:space:]]+//p' | head -1)
case $virt in "" | none) virt="no virtualisation detected" ;; *) virt="virtualised ($virt)" ;; esac
os=$(sed -nE 's/^PRETTY_NAME="?([^"]*)"?/\1/p' /etc/os-release 2>/dev/null | head -1)
version_header=lvgl-rust-sys/lvgl/include/lvgl/lv_version.h
lvgl=$(awk '/define LVGL_VERSION_(MAJOR|MINOR|PATCH)/ {v = v (v ? "." : "") $3} END {print v}' "$version_header")

configuration() { # slots features
    local text
    if [ "$1" = 0 ]; then text="views only"; else text="$1 handle slots"; fi
    [ "$2" = - ] || text="$text, \`${2//,/\`, \`}\`"
    echo "$text"
}

# --- The page ----------------------------------------------------------------

{
    cat <<EOF
<!-- Generated by \`mise run overhead\` (lvgl-rust/benches/overhead-report.sh). Do not edit by hand. -->

# Overhead measurements

This page is generated by \`mise run overhead\` and must not be edited by
hand: rerun the task to refresh it. It measures what lvgl-rust adds to raw
\`lvgl_rust::sys\` calls doing the same LVGL work, in time on the host and in
memory on two embedded targets. [Overhead](../explanation/overhead.md)
explains where the costs come from.

| | |
| --- | --- |
| Machine | $cpu, $(nproc) cores, $(uname -m), $virt |
| System | ${os:-unknown}, Linux $(uname -r) |
| Compiler | $(rustc --version) |
| LVGL | $lvgl |
| Commit | \`$(git describe --always --dirty)\` |
| Date | $(date -u +%Y-%m-%d) |

## Time per operation

Host build, release profile (\`cargo bench\`, feature \`heap\`), with the
vendored \`lvgl-rust/conf/lv_conf.h\` (LVGL's builtin heap, 32 handle
slots). Each cell is the median of 21 samples, each the mean of a batch of
5 000 or 100 000 operations after a warm-up batch, with the fastest
sample in brackets; the samples of the two paths of a row alternate, so
both see the same machine load. Raw calls include the crate's OBJ_ID hooks, which
LVGL runs for every object; "0" means the raw program does nothing for
that step.

EOF
    cat "$work/performance.md"
    cat <<'EOF'

The rows that drain (`Owned` dropped, `delete_later`, `post`) start and end
one session per operation, so they include the first row's cost.

## Memory

### Static memory in firmware

EOF
    if [ -s "$sizes" ]; then
        cat <<EOF
Firmware of \`lvgl-rust/size-gate\` (it initialises LVGL, creates, owns,
cleans and deletes objects, and runs the timer handler), size profile
(\`opt-level = "s"\`, no LTO), $size_source. Crate columns count the
crate's own symbols and the OBJ_ID hooks; firmware columns are the whole
image (\`size\`, Berkeley format: text includes read-only data), LVGL
included. The size gate checks the crate's \`.bss\` against the registry,
post queue and counters below, its \`.data\` (none), and its code against
the budget.

| Target | Configuration | Crate .bss, B | Crate .data, B | Crate code, B | Code budget, B | Firmware text, B | Firmware data, B | Firmware bss, B | Crate share of text |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
EOF
        while IFS=$'\t' read -r target slots features bss data code budget fw_text fw_data fw_bss _ _ _; do
            if [ "$fw_text" = - ]; then share=-; else share=$(awk -v c="$code" -v t="$fw_text" 'BEGIN {printf "%.1f %%", 100 * c / t}'); fi
            echo "| \`$target\` | $(configuration "$slots" "$features") | $bss | $data | $code | $budget | $fw_text | $fw_data | $fw_bss | $share |"
        done <"$sizes"
        cat <<'EOF'

Crate `.bss` by item, as the size gate checks it:

| Target | Configuration | Registry, B | `post` queue, B | Debug counters, B |
| --- | --- | ---: | ---: | ---: |
EOF
        while IFS=$'\t' read -r target slots features _ _ _ _ _ _ _ reg post counters; do
            echo "| \`$target\` | $(configuration "$slots" "$features") | $reg | $post | $counters |"
        done <"$sizes"
    else
        cat <<'EOF'
Not measured in this run: this machine has no thumbv6m or riscv32imc C
toolchain (`mise bootstrap packages apply` installs them), and no CI run
of the size gate matched the firmware's inputs.
EOF
    fi
    cat <<'EOF'

### Fixed and per-item costs

On 32-bit targets. N is `LVGL_RUST_HANDLE_SLOTS` (default 32, rounded up
to a power of two, 0 for views only) and P is `LVGL_RUST_POST_SLOTS`
(default 4).

| Item | Cost | Where |
| --- | --- | --- |
| Handle registry | 8·N + 24 B of `.bss` (280 B for N = 32); 8 B in views-only mode | `base/registry.rs`, checked by the size gate |
| `post` queue | 8·P + 4 B of `.bss` (36 B for P = 4); none for P = 0 | `base/post.rs`, checked by the size gate |
| Debug counters (feature `debug-counters`) | 8 B of `.bss` | `counters.rs`, checked by the size gate |
| Deferred-free list (feature `heap`) | 4 B of `.bss` | `base/deferred.rs` |
| Each LVGL object | 4 B (`lv_obj_t::id`, `LV_USE_OBJ_ID`), whether or not it has a handle | LVGL, needed by the registry |
| Each object or timer with a handle | one registry slot, no heap | `base/registry.rs` |
| `on`, `on_with`, `on_fn`, `on_static` | LVGL's event descriptor, as for a raw callback; no extra bytes | `event.rs` |
| `on_boxed` (feature `heap`) | one LVGL heap block: 8 B header, then the closure | `event.rs` (`BoxedHandler`) |
| `timer_boxed` (feature `heap`) | one LVGL heap block: 4 B header, then the closure | `timer.rs` (`BoxedTimer`) |
| `set_src_owned`, `set_src_buffer` (feature `heap`) | one LVGL heap block: 4 B header, then the image | `image.rs` |
| `subscribe_owned` (features `async` and `heap`) | one LVGL heap block: 4 B header, then the queue (indices, an `Option<Waker>`, the capture function, N events); the `OwnedQueue` receiver is 8 B | `queue.rs` |

LVGL's allocator adds its own block header to each heap block.

### Handle and view sizes

Measured on the host by the benchmark, which checks the 32-bit column
for the types that do not hold a pointer.

EOF
    cat "$work/types.md"
    cat <<'EOF'

## How to read these numbers

Host times are relative: they depend on the machine, so compare the
lvgl-rust and raw columns of one run, not runs on different machines. The
difference column is what the safe API adds to the LVGL work of the same
row; read it against the raw column, which is that work. Differences of a
nanosecond or less are within the noise of the measurement. Embedded
sizes are with the size profile and no LTO, so the crate's symbols stay
separate; in an application built with LTO, code it does not call is
dropped. Rerun `mise run overhead` after changing the crate or LVGL.
EOF
} >"$out"

echo "overhead: wrote ${out#"$root"/}"
