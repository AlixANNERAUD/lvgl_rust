"""Lays out the social banner around the rendered panel.

Usage: layout.py <panel.png> <banner.svg>
"""
import base64, html, sys

panel = base64.b64encode(open(sys.argv[1], 'rb').read()).decode()

K, T, F, N, C, P, S = '#c678dd', '#e5c07b', '#61afef', '#d19a66', '#7f848e', '#d4d4d8', '#98c379'
# Each line: list of (text, color).
code = [
    [('let ', K), ('arc', P), (' = ', P), ('Arc', T), ('::', P), ('create', F), ('(screen)?;', P)],
    [('arc', P), ('.', P), ('view', F), ('().', P), ('set_range', F), ('(', P), ('10', N), (', ', P), ('30', N), (');', P)],
    [('arc', P), ('.', P), ('view', F), ('().', P), ('set_value', F), ('(', P), ('21', N), (');', P)],
    [],
    [('let ', K), ('lights', P), (' = ', P), ('Switch', T), ('::', P), ('create', F), ('(screen)?;', P)],
    [('lights', P), ('.', P), ('view', F), ('().', P), ('on', F), ('(', P), ('EventCode', T), ('::', P), ('VALUE_CHANGED', N), (', |e| {', P)],
    [('    // Handlers get shared access:', C)],
    [('    // exclusive work is posted.', C)],
    [('    ', P), ('let ', K), ('_ = e.', P), ('lv', F), ('().', P), ('post', F), ('(toggle_lights, ());', P)],
    [('})?;', P)],
]

def code_svg(x, y, lh):
    out = []
    for i, line in enumerate(code):
        if not line: continue
        spans = ''.join(f'<tspan fill="{c}">{html.escape(t)}</tspan>' for t, c in line)
        out.append(f'<text x="{x}" y="{y + i*lh}" xml:space="preserve" font-family="JetBrains Mono" font-size="15.5">{spans}</text>')
    return '\n'.join(out)

chips = ['No allocator required', 'Use-after-free proof handles', 'Generated from your lv_conf.h', 'Miri + loom tested']
def chips_svg(x, y):
    out, cx = [], x
    for c in chips:
        w = 30 + len(c) * 7.6
        out.append(f'<rect x="{cx}" y="{y}" width="{w:.0f}" height="32" rx="16" fill="#23262b" stroke="#33363c"/>'
                   f'<text x="{cx + w/2:.0f}" y="{y+21}" text-anchor="middle" font-family="Inter" font-weight="500" font-size="14.5" fill="#c9cdd3">{c}</text>')
        cx += w + 10
    return '\n'.join(out)

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1280" height="640" viewBox="0 0 1280 640">
<rect width="1280" height="640" fill="#111214"/>
<text x="80" y="98" font-family="Inter" font-weight="700" font-size="62" fill="#f2f2f2" letter-spacing="-1.5">lvgl-rust</text>
<text x="1200" y="88" text-anchor="end" font-family="Inter" font-size="23" fill="#c4c4c4">no_std · ESP32 · STM32 · RISC-V · WebAssembly</text>
<rect x="110" y="124" width="1060" height="482" rx="16" fill="#1a1b1e" stroke="#2b2d31"/>
<text x="146" y="172" font-family="Inter" font-weight="600" font-size="25" fill="#f2f2f2">Safe Rust in, LVGL UI out</text>
<text x="146" y="201" font-family="Inter" font-size="17" fill="#a9adb3">Memory-safe bindings to LVGL 9, the embedded GUI library, for microcontrollers, WebAssembly and desktop</text>
<rect x="146" y="226" width="482" height="316" rx="12" fill="#141518" stroke="#26282c"/>
<circle cx="168" cy="246" r="5" fill="#ff5f57"/><circle cx="186" cy="246" r="5" fill="#febc2e"/><circle cx="204" cy="246" r="5" fill="#28c840"/>
<text x="608" y="251" text-anchor="end" font-family="JetBrains Mono" font-size="13" fill="#6b7078">main.rs</text>
{code_svg(166, 288, 25)}
<rect x="656" y="226" width="496" height="316" rx="16" fill="#0b0d10" stroke="#3a3d43" stroke-width="2"/>
<clipPath id="screen"><rect x="664" y="234" width="480" height="300" rx="9"/></clipPath>
<image x="664" y="234" width="480" height="300" clip-path="url(#screen)" xlink:href="data:image/png;base64,{panel}"/>
{chips_svg(146, 558)}
</svg>'''
open(sys.argv[2], 'w').write(svg)
