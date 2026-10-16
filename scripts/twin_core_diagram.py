"""Generate docs/twin-core/twin-core-signal-flow.svg. Run from the repo root:

    python3 scripts/twin_core_diagram.py
"""

from html import escape

W, H = 1040, 1340
CX = 380            # main column centre
BW = 330            # main box width
NX = 865            # noise column centre
NW = 270

CAT = {
    "trigger": ("#fff4d6", "#d08a00", "#7a5000"),
    "resonator": ("#e0ecff", "#2f6fdb", "#163d82"),
    "shaping": ("#efe7ff", "#7a4fd6", "#43278a"),
    "dynamics": ("#ddf5e5", "#1f9d55", "#0f5c31"),
    "noise": ("#eef1f5", "#6b7a90", "#323c4c"),
}
XMOD = "#0f9488"
CHAOS = "#d6365a"
EDGE = "#3d4656"

out = []
add = out.append


def text(x, y, s, cls, anchor="middle"):
    add(f'<text x="{x}" y="{y}" class="{cls}" text-anchor="{anchor}">{escape(s)}</text>')


def node(num, cx, y, w, h, cat, title, role, params, new=False):
    fill, stroke, ink = CAT[cat]
    x = cx - w / 2
    add(f'<g class="node" id="node-{num}">')
    add(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="{fill}" '
        f'stroke="{stroke}" stroke-width="2"/>')
    add(f'<circle cx="{x + 24}" cy="{y + 24}" r="13" fill="{stroke}"/>')
    text(x + 24, y + 29, str(num), "badge")
    add(f'<text x="{x + 46}" y="{y + 30}" class="title" fill="{ink}">{escape(title)}</text>')
    if new:
        nx = x + w - 52
        add(f'<rect x="{nx}" y="{y + 12}" width="40" height="20" rx="10" fill="{CHAOS}"/>')
        text(nx + 20, y + 26, "NEW", "new")
    text(x + 18, y + 54, role, "role", "start")
    for i, line in enumerate(params):
        text(x + 18, y + 76 + i * 18, line, "param", "start")
    add("</g>")


def pill(cx, cy, w, label, fill="#3d4656"):
    add(f'<rect x="{cx - w / 2}" y="{cy - 18}" width="{w}" height="36" rx="18" fill="{fill}"/>')
    text(cx, cy + 5, label, "pill")


def summer(cx, cy):
    add(f'<circle cx="{cx}" cy="{cy}" r="17" fill="#ffffff" stroke="{EDGE}" stroke-width="2"/>')
    text(cx, cy + 6, "+", "sum")


def path(d, color=EDGE, dash=None, marker="arrow", width=2):
    dash_attr = f' stroke-dasharray="{dash}"' if dash else ""
    add(f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}"'
        f'{dash_attr} marker-end="url(#{marker})"/>')


def label(x, y, s, color=EDGE, anchor="start"):
    add(f'<text x="{x}" y="{y}" class="edge" fill="{color}" text-anchor="{anchor}">'
        f'{escape(s)}</text>')


# Vertical layout of the main column (top y of each box, height).
BOX_H = 92
Y = {1: 178, 2: 300, 3: 470, 4: 612, 5: 760, 6: 882, 7: 1004, 8: 1126}
SUM1 = 428
SUM2 = 1262
NOISE_Y, NOISE_H = 178, 110
TRIG_Y = 116
LEFT = CX - BW / 2
RIGHT = CX + BW / 2

add(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" '
    f'height="{H}" role="img" aria-labelledby="t d">')
add('<title id="t">Twin Core percussion voice: signal flow</title>')
add('<desc id="d">A trigger fires a delayed exciter pulse into two serial resonator '
    'cores. Core one can frequency-modulate core two, and core two can feed back '
    'into core one when chaos is above one half. The body passes through a mode '
    'tap, a wavefolder, a VCA and a vactrol low-pass gate, then sums with an '
    'independently enveloped noise path, which can instead excite the cores.</desc>')
add("""<defs>
<style>
text { font-family: Inter, "SF Pro Text", "Helvetica Neue", Arial, sans-serif; }
.h1 { font-size: 26px; font-weight: 700; fill: #1d2330; }
.sub { font-size: 14px; fill: #5b6576; }
.title { font-size: 17px; font-weight: 650; }
.role { font-size: 13px; fill: #3d4656; }
.param { font-family: "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace;
         font-size: 12px; fill: #4a5568; }
.badge { font-size: 13px; font-weight: 700; fill: #ffffff; }
.new { font-size: 11px; font-weight: 700; fill: #ffffff; letter-spacing: 0.5px; }
.pill { font-size: 14px; font-weight: 600; fill: #ffffff; }
.sum { font-size: 22px; font-weight: 600; fill: #3d4656; }
.edge { font-size: 12.5px; font-weight: 600; }
.legend { font-size: 13px; fill: #3d4656; }
</style>""")
for name, color in [("arrow", EDGE), ("arrow-x", XMOD), ("arrow-c", CHAOS)]:
    add(f'<marker id="{name}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" '
        f'markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" '
        f'fill="{color}"/></marker>')
add("</defs>")
add(f'<rect width="{W}" height="{H}" rx="24" fill="#fbfaf7"/>')
text(40, 52, "Twin Core percussion voice", "h1", "start")
text(40, 78, "Signal flow from trigger to output. Numbers match the node notes.",
     "sub", "start")

# --- Edges (drawn first so boxes sit on top) ---------------------------------
# TRIG to body and noise.
path(f"M{CX},{TRIG_Y + 18} V{Y[1] - 4}")
path(f"M{CX + 55},{TRIG_Y} H{NX - 50} V{NOISE_Y - 4}")
label(CX + 70, TRIG_Y - 8, "same trigger fires the noise")
# N-TRIG into noise.
path(f"M{NX + 85},{TRIG_Y + 18} V{NOISE_Y - 4}")
# Body chain.
path(f"M{CX},{Y[1] + BOX_H} V{Y[2] - 4}")
path(f"M{CX},{Y[2] + BOX_H} V{SUM1 - 21}")
path(f"M{CX},{SUM1 + 17} V{Y[3] - 4}")
label(CX + 10, SUM1 + 34, "strike + body noise + chaos")
path(f"M{CX},{Y[3] + BOX_H} V{Y[4] - 4}")
label(CX + 10, Y[3] + BOX_H + 30, "low-pass (cores in series)")
path(f"M{CX},{Y[4] + BOX_H} V{Y[5] - 4}")
label(CX + 10, Y[4] + BOX_H + 32, "low-pass and band-pass taps")
for a, b in [(5, 6), (6, 7), (7, 8)]:
    path(f"M{CX},{Y[a] + BOX_H} V{Y[b] - 4}")
path(f"M{CX},{Y[8] + BOX_H} V{SUM2 - 21}")
# Output.
path(f"M{CX},{SUM2 + 17} V{SUM2 + 30} H{CX + 150}")
pill(CX + 285, SUM2 + 30, 250, "× volume × √velocity → out", "#1d2330")

# Noise routes: BODY into the first summer, LP/HP to the output summer.
BODY_X, DIRECT_X = NX - 80, NX + 80
path(f"M{BODY_X},{NOISE_Y + NOISE_H} V{SUM1} H{CX + 21}", CAT["noise"][1])
label(BODY_X - 8, NOISE_Y + NOISE_H + 26, "BODY mode:", CAT["noise"][1], "end")
label(BODY_X - 8, NOISE_Y + NOISE_H + 42, "low-passed noise", CAT["noise"][1], "end")
label(BODY_X - 8, NOISE_Y + NOISE_H + 58, "excites the cores", CAT["noise"][1], "end")
path(f"M{DIRECT_X},{NOISE_Y + NOISE_H} V{SUM2} H{CX + 21}", CAT["noise"][1])
label(DIRECT_X - 8, NOISE_Y + NOISE_H + 26, "LP / HP mode:", CAT["noise"][1], "end")
label(DIRECT_X - 8, NOISE_Y + NOISE_H + 42, "mixed straight", CAT["noise"][1], "end")
label(DIRECT_X - 8, NOISE_Y + NOISE_H + 58, "to the output", CAT["noise"][1], "end")

# Cross mod: core 1 level bends core 2 pitch (right side).
XM_X = RIGHT + 70
path(f"M{RIGHT},{Y[3] + 46} H{XM_X} V{Y[4] + 46} H{RIGHT + 4}", XMOD, "2 5", "arrow-x", 2.5)
label(XM_X + 10, Y[3] + 112, "cross mod", XMOD)
label(XM_X + 10, Y[3] + 128, "core 1 bends", XMOD)
label(XM_X + 10, Y[3] + 144, "core 2 pitch", XMOD)

# Chaos feedback: core 2 band-pass back into the first summer (left side).
CH_X = LEFT - 70
path(f"M{LEFT},{Y[4] + 46} H{CH_X} V{SUM1} H{CX - 21}", CHAOS, "8 6", "arrow-c", 2.5)
label(CH_X - 10, Y[3] + 30, "chaos > 0.5", CHAOS, "end")
label(CH_X - 10, Y[3] + 46, "core 2 band-pass", CHAOS, "end")
label(CH_X - 10, Y[3] + 62, "fed back through", CHAOS, "end")
label(CH_X - 10, Y[3] + 78, "a saturator,", CHAOS, "end")
label(CH_X - 10, Y[3] + 94, "gated by the", CHAOS, "end")
label(CH_X - 10, Y[3] + 110, "body envelope", CHAOS, "end")

# --- Nodes --------------------------------------------------------------------
pill(CX, TRIG_Y, 110, "TRIG")
pill(NX + 85, TRIG_Y, 110, "N-TRIG", "#6b7a90")
node(1, CX, Y[1], BW, BOX_H, "trigger", "Trigger delay",
     "Holds back the body; the noise fires on time", ["trigger_delay"])
node(2, CX, Y[2], BW, BOX_H, "trigger", "Exciter",
     "A 0.75 ms pulse, scaled by velocity", ["velocity"])
summer(CX, SUM1)
node(3, CX, Y[3], BW, BOX_H, "resonator", "Core 1",
     "Struck resonator; pitch sweep gives the punch",
     ["tune · fm_depth · fm_decay · chaos"], new=True)
node(4, CX, Y[4], BW, BOX_H, "resonator", "Core 2",
     "Second resonator, tuned below core 1",
     ["detune · cross_mod · chaos"], new=True)
node(5, CX, Y[5], BW, BOX_H, "shaping", "Body mode tap",
     "Picks which filter outputs make the body",
     ["body_mode: LOW · MID · HIGH"])
node(6, CX, Y[6], BW, BOX_H, "shaping", "Wavefolder",
     "Folds the body for extra harmonics (2×)",
     ["harmonics · symmetry"], new=True)
node(7, CX, Y[7], BW, BOX_H, "dynamics", "Body VCA",
     "Decay envelope for the whole body",
     ["length · body_bias"])
node(8, CX, Y[8], BW, BOX_H, "dynamics", "Vactrol LPG",
     "Low-pass gate: darkens as it fades",
     ["lpg"], new=True)
summer(CX, SUM2)
node(9, NX, NOISE_Y, NW, NOISE_H, "noise", "Noise",
     "Filtered noise with its own envelope",
     ["noise_filter · noise_decay", "noise_bias · noise_mode"])

# --- Legend ---------------------------------------------------------------------
LX, LY = 640, 800
add(f'<rect x="{LX - 22}" y="{LY - 34}" width="290" height="250" rx="14" fill="#ffffff" '
    f'stroke="#d9dde4"/>')
text(LX, LY - 8, "Key", "title", "start")
rows = [
    ("trigger", "Trigger and excitation"),
    ("resonator", "Resonant cores"),
    ("shaping", "Tone shaping"),
    ("dynamics", "Dynamics"),
    ("noise", "Noise path"),
]
for i, (cat, name) in enumerate(rows):
    fill, stroke, _ = CAT[cat]
    y = LY + 18 + i * 25
    add(f'<rect x="{LX}" y="{y - 12}" width="24" height="16" rx="4" fill="{fill}" '
        f'stroke="{stroke}" stroke-width="2"/>')
    text(LX + 36, y + 1, name, "legend", "start")
y = LY + 18 + 5 * 25
add(f'<path d="M{LX},{y - 4} h26" stroke="{XMOD}" stroke-width="2.5" stroke-dasharray="2 5"/>')
text(LX + 36, y + 1, "Audio-rate pitch modulation", "legend", "start")
y += 25
add(f'<path d="M{LX},{y - 4} h26" stroke="{CHAOS}" stroke-width="2.5" stroke-dasharray="8 6"/>')
text(LX + 36, y + 1, "Chaos feedback", "legend", "start")
y += 25
add(f'<rect x="{LX - 4}" y="{y - 14}" width="34" height="18" rx="9" fill="{CHAOS}"/>')
text(LX + 13, y - 1, "NEW", "new")
text(LX + 36, y + 1, "Beyond the hardware design", "legend", "start")

add("</svg>")
open("docs/twin-core/twin-core-signal-flow.svg", "w").write("\n".join(out) + "\n")
