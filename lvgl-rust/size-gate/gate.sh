#!/usr/bin/env bash
# Size gate for lvgl-rust: builds the firmware in this directory for an
# embedded target and checks the crate's footprint in the linked image.
#
#   gate.sh <target> <slots> <text budget in bytes> [firmware features: defmt, debug-counters]
#
# Checks: the registry static is 8·N + 24 bytes in .bss (8 bytes with N = 0),
# and the `post` queue 8·P + 4 bytes (none with P = 0,
# LVGL_RUST_POST_SLOTS, default 4), plus the 8 bytes of counters with
# `debug-counters` (and nothing else from them without it);
# the crate has no .data; no core::fmt or panic code is linked; the OBJ_ID
# hooks are defined with N > 0 and absent with N = 0; the crate's code fits
# the budget. The C toolchain comes from the `size-gate` mise task.
#
# It also prints the whole firmware's sizes (LVGL included), and with
# SIZE_GATE_REPORT set to a file appends one tab-separated line there:
# target, slots, features, crate .bss, .data and code, budget, firmware
# text, data and bss (`size`, Berkeley format), registry, post queue and
# counters bytes (read by `mise run overhead`).
set -euo pipefail

target=$1
slots=$2
budget=$3
features=${4:-}
root=$(cd "$(dirname "$0")/../.." && pwd)

if [ "$slots" = 0 ]; then
    conf=$root/lvgl-rust-sys/conf # keeps LVGL's own OBJ_ID support
else
    conf=$root/lvgl-rust/conf
fi
dir=$root/target/size-gate/n$slots
feature_args=()
if [ -n "$features" ]; then
    dir=$dir-${features//,/-}
    feature_args=(--features "$features") # forwarded by the firmware's Cargo.toml
fi
DEP_LV_CONFIG_PATH=$conf LVGL_RUST_HANDLE_SLOTS=$slots CARGO_TARGET_DIR=$dir \
    cargo build --quiet -p lvgl-rust-size-gate --profile size --target "$target" "${feature_args[@]}"
elf=$dir/$target/size/lvgl-rust-size-gate

case $target in
thumb*) nm=arm-none-eabi-nm size_tool=arm-none-eabi-size ;;
riscv*) nm=riscv64-unknown-elf-nm size_tool=riscv64-unknown-elf-size ;;
*) echo "unsupported target $target" >&2 && exit 2 ;;
esac

label="$target, N = $slots${features:+, $features}"
fail() {
    echo "size gate ($label): $*" >&2
    exit 1
}

hooks='lv_obj_assign_id|lv_obj_free_id|lv_obj_set_id|lv_obj_id_compare|lv_obj_stringify_id'
# "address size type name", sized symbols only, one line per address.
symbols=$("$nm" -S -C "$elf" | awk 'NF >= 4' | sort -u -k1,1)
crate=$(grep -E "^\S+ \S+ \S (<)?lvgl_rust::|^\S+ \S+ \S ($hooks)$" <<<"$symbols" || true)

if [ "$slots" = 0 ]; then expected_reg=8; else expected_reg=$((8 * slots + 24)); fi
reg=$(grep -E ' lvgl_rust::base::registry::REG$' <<<"$symbols" || fail "registry static not found")
reg_size=$((16#$(awk '{print $2}' <<<"$reg")))
reg_type=$(awk '{print $3}' <<<"$reg")
[ "$reg_size" = "$expected_reg" ] || fail "registry is $reg_size B, expected $expected_reg B"
[[ $reg_type =~ ^[bB]$ ]] || fail "registry is in section type '$reg_type', expected .bss"

data=$(awk '$3 ~ /^[dD]$/' <<<"$crate")
[ -z "$data" ] || fail "crate data in .data: $data"

post_slots=${LVGL_RUST_POST_SLOTS:-4}
if [ "$post_slots" = 0 ]; then expected_post=0; else expected_post=$((8 * post_slots + 4)); fi
counters=$(grep -E ' lvgl_rust::counters::enabled::COUNTERS$' <<<"$symbols" || true)
if [[ ",$features," == *,debug-counters,* ]]; then
    expected_counters=8
    [ -n "$counters" ] || fail "debug counters not found"
    counters_size=$((16#$(awk '{print $2}' <<<"$counters")))
    [ "$counters_size" = 8 ] || fail "debug counters are $counters_size B, expected 8 B"
else
    expected_counters=0
    [ -z "$counters" ] || fail "debug counters linked without debug-counters"
fi
expected_bss=$((expected_reg + expected_post + expected_counters))
bss=0
while read -r _ size _ _; do bss=$((bss + 16#$size)); done < <(awk '$3 ~ /^[bB]$/' <<<"$crate")
[ "$bss" = "$expected_bss" ] || fail "crate .bss is $bss B, expected $expected_bss B (registry $expected_reg, post queue $expected_post, counters $expected_counters)"

panics=$("$nm" -C "$elf" | grep -E 'core::fmt|core::panicking|unwrap_failed' || true)
[ -z "$panics" ] || fail "formatting or panic code linked: $panics"

defined_hooks=$(grep -cE " [tT] ($hooks)$" <<<"$symbols" || true)
if [ "$slots" = 0 ]; then
    [ "$defined_hooks" = 0 ] || fail "views-only mode defines OBJ_ID hooks"
else
    [ "$defined_hooks" = 5 ] || fail "$defined_hooks of the 5 OBJ_ID hooks are defined"
fi

text=0
while read -r _ size _ _; do text=$((text + 16#$size)); done < <(awk '$3 ~ /^[tTrR]$/' <<<"$crate")
echo "size gate ($label): .bss $bss B, .data 0 B, code $text B (budget $budget B)"
read -r fw_text fw_data fw_bss _ < <("$size_tool" "$elf" | awk 'NR == 2')
echo "size gate ($label): firmware text $fw_text B, data $fw_data B, bss $fw_bss B (LVGL included)"
if [ -n "${SIZE_GATE_REPORT:-}" ]; then
    printf '%s\t%s\t%s\t%s\t0\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$target" "$slots" "${features:--}" \
        "$bss" "$text" "$budget" "$fw_text" "$fw_data" "$fw_bss" "$expected_reg" "$expected_post" \
        "$expected_counters" >>"$SIZE_GATE_REPORT"
fi
[ "$text" -le "$budget" ] || fail "code is $text B, over the $budget B budget"
