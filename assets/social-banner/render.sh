#!/bin/sh
# Renders social-banner.png, the repository's GitHub social preview (upload
# it in Settings > General > Social preview). Run through `mise run
# social-banner` from the repository root.
#
# 1. The panel on the right is real LVGL output: `screen/` builds it with
#    lvgl-rust (dark theme, larger Montserrat fonts) and writes a PNG.
# 2. `layout.py` places the panel, the code snippet (compiled in
#    `screen/src/main.rs`, `banner_snippet`) and the text in an SVG.
# 3. resvg renders the SVG with Inter and JetBrains Mono.
set -eu

here=assets/social-banner
work=target/social-banner
mkdir -p "$work/conf" "$work/fonts"

sed -E \
  -e 's/^#define LV_FONT_MONTSERRAT_(16|20|28|40) +0/#define LV_FONT_MONTSERRAT_\1 1/' \
  -e 's/^( *#define LV_THEME_DEFAULT_DARK) +0/\1 1/' \
  lvgl-rust/conf/lv_conf.h > "$work/conf/lv_conf.h"

DEP_LV_CONFIG_PATH="$PWD/$work/conf" CARGO_TARGET_DIR="$PWD/$work/target" \
  cargo run -q --manifest-path "$here/screen/Cargo.toml" -- "$work/panel.png"

if [ ! -f "$work/fonts/Inter-Bold.ttf" ]; then
  curl -sSL -o "$work/inter.zip" https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
  curl -sSL -o "$work/jetbrains-mono.zip" https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
  python3 -I - "$work" <<'PY'
import sys, zipfile
work = sys.argv[1]
wanted = {"Inter-Regular.ttf", "Inter-Medium.ttf", "Inter-SemiBold.ttf", "Inter-Bold.ttf",
          "JetBrainsMono-Regular.ttf"}
for archive in ("inter.zip", "jetbrains-mono.zip"):
    with zipfile.ZipFile(f"{work}/{archive}") as z:
        for name in z.namelist():
            base = name.rsplit("/", 1)[-1]
            if base in wanted and "variable" not in name.lower():
                open(f"{work}/fonts/{base}", "wb").write(z.read(name))
PY
fi

python3 -I "$here/layout.py" "$work/panel.png" "$work/social-banner.svg"
resvg --use-fonts-dir "$work/fonts" --skip-system-fonts "$work/social-banner.svg" "$here/social-banner.png"
echo "wrote $here/social-banner.png"
