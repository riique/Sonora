# Gera os banners SVG do README (claro e escuro) a partir dos tokens do DESIGN.md.
# Uso: python3 docs/assets/generate.py
import os
OUT = os.path.dirname(os.path.abspath(__file__))
SANS = "'Segoe UI Variable Display','Segoe UI',system-ui,-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif"
TEXT = "'Segoe UI Variable Text','Segoe UI',system-ui,-apple-system,BlinkMacSystemFont,'Helvetica Neue',Arial,sans-serif"
MONO = "'Cascadia Mono','Cascadia Code',Consolas,'SFMono-Regular',Menlo,monospace"

T = {
 "light": dict(bg="#f8f8f6", raised="#ffffff", fill="#e9e9e5", hair="#dfdfda", hair2="#cacac4",
               faint="#a3a39d", muted="#67675f", soft="#4b4b47", ink="#171716", lamp="#e5483f", red="#c4322b", green="#276b41"),
 "dark":  dict(bg="#161615", raised="#232322", fill="#262625", hair="#31312f", hair2="#454543",
               faint="#6a6a66", muted="#a3a39d", soft="#c4c4be", ink="#f2f2ee", lamp="#ff6b61", red="#ff6b61", green="#5cc283"),
}

def esc(s): return s.replace("&","&amp;").replace("<","&lt;")

def bars(x0, cy, heights, w=3, gap=4, color="#f2f2ee"):
    out=[]
    for i,h in enumerate(heights):
        x = x0 + i*(w+gap)
        out.append(f'<rect x="{x}" y="{cy-h/2:.1f}" width="{w}" height="{h}" rx="{w/2}" fill="{color}"/>')
    return "\n".join(out)

def keycap(x, y, label, c, h=30, pad=11, cw=8.4, size=13):
    w = pad*2 + cw*len(label)
    return (f'<rect x="{x}" y="{y+1.5}" width="{w}" height="{h}" rx="7" fill="{c["hair2"]}"/>'
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="7" fill="{c["raised"]}" stroke="{c["hair2"]}"/>'
            f'<text x="{x+w/2}" y="{y+h/2+4.5}" text-anchor="middle" font-family="{MONO}" font-size="{size}" fill="{c["ink"]}">{label}</text>'), w

def gadget(cx, cy, scale=1.4):
    # recording pill, scaled from the app's 170x36 .sonora-bar--recording
    w, h = 170*scale, 36*scale
    x, y = cx-w/2, cy-h/2
    heights = [6,10,16,24,14,30,20,12,26,18,9,15,22,11,7]
    g = [f'<g filter="url(#pill)"><rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{h/2}" fill="#161615"/></g>',
         f'<rect x="{x+.5}" y="{y+.5}" width="{w-1}" height="{h-1}" rx="{h/2}" fill="none" stroke="#ffffff" stroke-opacity=".08"/>',
         f'<circle cx="{x+h/2}" cy="{cy}" r="11" fill="#e5483f" fill-opacity=".18"/>',
         f'<circle cx="{x+h/2}" cy="{cy}" r="5.5" fill="#e5483f"/>',
         bars(x+h-2, cy, [v*0.82 for v in heights], w=3, gap=3.6, color="#e2e2dd"),
         f'<text x="{x+w-18}" y="{cy+4.5}" text-anchor="end" font-family="{MONO}" font-size="13" fill="#a3a39d">0:07</text>']
    return "\n".join(g)

def defs(c):
    return f'''<defs>
  <filter id="pill" x="-20%" y="-60%" width="140%" height="240%">
    <feDropShadow dx="0" dy="14" stdDeviation="14" flood-color="#000" flood-opacity=".38"/>
  </filter>
  <filter id="win" x="-10%" y="-10%" width="120%" height="130%">
    <feDropShadow dx="0" dy="18" stdDeviation="18" flood-color="#141412" flood-opacity="{'.10' if c['bg']=='#f8f8f6' else '.45'}"/>
  </filter>
</defs>'''

def hero(name, c):
    W,H = 1280, 480
    s=[f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="Sonora — ditado por voz para Windows">',
       defs(c),
       f'<rect width="{W}" height="{H}" rx="20" fill="{c["bg"]}"/>',
       f'<rect x=".5" y=".5" width="{W-1}" height="{H-1}" rx="19.5" fill="none" stroke="{c["hair"]}"/>']
    # left column
    s.append(f'<circle cx="104" cy="139" r="16" fill="{c["lamp"]}" fill-opacity=".16"/>')
    s.append(f'<circle cx="104" cy="139" r="8" fill="{c["lamp"]}"/>')
    s.append(f'<text x="132" y="148" font-family="{TEXT}" font-size="15" font-weight="600" fill="{c["red"]}" letter-spacing=".2">No ar</text>')
    s.append(f'<text x="86" y="258" font-family="{SANS}" font-size="112" font-weight="600" letter-spacing="-3" fill="{c["ink"]}">Sonora</text>')
    s.append(f'<text x="90" y="312" font-family="{TEXT}" font-size="25" fill="{c["soft"]}">Fale em qualquer aplicativo.</text>')
    s.append(f'<text x="90" y="346" font-family="{TEXT}" font-size="25" fill="{c["muted"]}">O texto chega pronto no campo em foco.</text>')
    s.append(f'<line x1="90" y1="384" x2="560" y2="384" stroke="{c["hair"]}"/>')
    s.append(f'<text x="90" y="412" font-family="{MONO}" font-size="13.5" fill="{c["muted"]}">v2.0.1   ·   Windows   ·   Tauri 2 + Rust</text>')
    # right: target window
    wx, wy, ww, wh = 672, 76, 520, 256
    s.append(f'<g filter="url(#win)"><rect x="{wx}" y="{wy}" width="{ww}" height="{wh}" rx="12" fill="{c["raised"]}"/></g>')
    s.append(f'<rect x="{wx+.5}" y="{wy+.5}" width="{ww-1}" height="{wh-1}" rx="11.5" fill="none" stroke="{c["hair"]}"/>')
    s.append(f'<text x="{wx+24}" y="{wy+32}" font-family="{TEXT}" font-size="13" fill="{c["muted"]}">notas-da-reuniao.md</text>')
    for i,dx in enumerate([0,18,36]):
        s.append(f'<line x1="{wx+ww-62+dx}" y1="{wy+27}" x2="{wx+ww-54+dx}" y2="{wy+27}" stroke="{c["faint"]}" stroke-width="1.4" stroke-linecap="round"/>')
    s.append(f'<line x1="{wx}" y1="{wy+52}" x2="{wx+ww}" y2="{wy+52}" stroke="{c["hair"]}"/>')
    lines = [("Revisar o pipeline Preciso antes da release", c["muted"]),
             ("e mover o fallback para o Whisper turbo.", c["muted"]),
             ("Depois, conferir o @src/views/HistoricoView.tsx", c["ink"])]
    for i,(t,col) in enumerate(lines):
        s.append(f'<text x="{wx+24}" y="{wy+94+i*34}" font-family="{TEXT}" font-size="18" fill="{col}">{esc(t)}</text>')
    # highlight last line as freshly pasted
    s.append(f'<rect x="{wx+16}" y="{wy+150}" width="4" height="0" fill="none"/>')
    s.append(f'<rect x="{wx+24}" y="{wy+180}" width="2" height="22" rx="1" fill="{c["ink"]}"/>')
    # pasted chip
    s.append(f'<path d="M{wx+25} {wy+226} l4 4 l8 -9" fill="none" stroke="{c["green"]}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>')
    s.append(f'<text x="{wx+44}" y="{wy+232}" font-family="{MONO}" font-size="12.5" fill="{c["green"]}">colado no campo em foco</text>')
    # gadget + keycaps
    s.append(gadget(wx+ww/2, 384, 1.42))
    k1, w1 = keycap(wx+ww/2-58, 424, "Ctrl", c, h=26, pad=9, cw=8, size=12)
    s.append(k1)
    s.append(f'<text x="{wx+ww/2-58+w1+10}" y="{442}" font-family="{MONO}" font-size="13" fill="{c["faint"]}">+</text>')
    k2, _ = keycap(wx+ww/2-58+w1+24, 424, "B", c, h=26, pad=9, cw=8, size=12)
    s.append(k2)
    s.append('</svg>')
    open(f"{OUT}/{name}", "w").write("\n".join(s))

def flow(name, c):
    W,H = 1280, 300
    s=[f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="Fluxo: atalho, fala, pipeline, texto colado e histórico">',
       defs(c), f'<rect width="{W}" height="{H}" rx="20" fill="{c["bg"]}"/>',
       f'<rect x=".5" y=".5" width="{W-1}" height="{H-1}" rx="19.5" fill="none" stroke="{c["hair"]}"/>']
    xs = [140, 395, 640, 885, 1140]
    cy = 128
    s.append(f'<line x1="{xs[0]}" y1="{cy}" x2="{xs[-1]}" y2="{cy}" stroke="{c["hair2"]}" stroke-dasharray="2 6" stroke-linecap="round"/>')
    titles = ["Atalho", "Você fala", "Pipeline", "Colado", "Histórico"]
    subs = [["Ctrl+B em qualquer", "aplicativo"], ["captura local,", "silêncio filtrado"], ["Whisper · Gemini", "+ vocabulário"],
            ["Ctrl+V no campo", "que estava em foco"], ["local, com áudio,", "retranscrição e retry"]]
    times = ["1", "2", "3", "4", "5"]
    for i,x in enumerate(xs):
        # station glyph
        s.append(f'<rect x="{x-44}" y="{cy-44}" width="88" height="88" rx="22" fill="{c["raised"]}" stroke="{c["hair"]}"/>')
        if i==0:
            k,w = keycap(x-34, cy-15, "Ctrl+B", c, h=28, pad=9, cw=8.2, size=13); s.append(k)
        elif i==1:
            s.append(bars(x-26, cy, [8,16,28,20,34,14,24,10], w=3.4, gap=3.4, color=c["ink"]))
        elif i==2:
            for j,(dy,wid) in enumerate([(-14,40),(0,28),(14,36)]):
                s.append(f'<line x1="{x-20}" y1="{cy+dy}" x2="{x-20+wid}" y2="{cy+dy}" stroke="{c["ink"]}" stroke-width="3" stroke-linecap="round"/>')
                s.append(f'<circle cx="{x-20+wid}" cy="{cy+dy}" r="3.6" fill="{c["raised"]}" stroke="{c["ink"]}" stroke-width="2"/>')
        elif i==3:
            s.append(f'<rect x="{x-22}" y="{cy-18}" width="44" height="36" rx="6" fill="none" stroke="{c["ink"]}" stroke-width="2.2"/>')
            s.append(f'<line x1="{x-12}" y1="{cy-4}" x2="{x+8}" y2="{cy-4}" stroke="{c["ink"]}" stroke-width="2.2" stroke-linecap="round"/>')
            s.append(f'<line x1="{x-12}" y1="{cy+6}" x2="{x+2}" y2="{cy+6}" stroke="{c["ink"]}" stroke-width="2.2" stroke-linecap="round"/>')
            s.append(f'<rect x="{x+5}" y="{cy}" width="2.2" height="12" fill="{c["ink"]}"/>')
        else:
            for j in range(3):
                yy = cy-14+j*14
                s.append(f'<line x1="{x-22}" y1="{yy}" x2="{x-12}" y2="{yy}" stroke="{c["faint"]}" stroke-width="2.2" stroke-linecap="round"/>')
                s.append(f'<line x1="{x-4}" y1="{yy}" x2="{x+22}" y2="{yy}" stroke="{c["ink"]}" stroke-width="2.2" stroke-linecap="round"/>')
        s.append(f'<text x="{x}" y="{cy+76}" text-anchor="middle" font-family="{SANS}" font-size="19" font-weight="600" fill="{c["ink"]}">{titles[i]}</text>')
        for j,t in enumerate(subs[i]):
            s.append(f'<text x="{x}" y="{cy+100+j*20}" text-anchor="middle" font-family="{TEXT}" font-size="14" fill="{c["muted"]}">{esc(t)}</text>')
        pass
    s.append('</svg>')
    open(f"{OUT}/{name}", "w").write("\n".join(s))

def modes(name, c):
    W,H = 1280, 250
    s=[f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="Os quatro pipelines, do mais rápido ao mais preciso">',
       f'<rect width="{W}" height="{H}" rx="20" fill="{c["bg"]}"/>',
       f'<rect x=".5" y=".5" width="{W-1}" height="{H-1}" rx="19.5" fill="none" stroke="{c["hair"]}"/>']
    x0,x1,y = 110, 1170, 132
    s.append(f'<line x1="{x0}" y1="{y}" x2="{x1}" y2="{y}" stroke="{c["hair2"]}" stroke-width="2"/>')
    s.append(f'<text x="{x0}" y="{y+82}" font-family="{MONO}" font-size="12.5" fill="{c["muted"]}">← mais rápido</text>')
    s.append(f'<text x="{x1}" y="{y+82}" text-anchor="end" font-family="{MONO}" font-size="12.5" fill="{c["muted"]}">mais preciso →</text>')
    data = [("Ultrarrápido","Whisper via Groq"),("Rápido e preciso","Gemini multimodal"),("Preciso","Whisper ∥ Gemini refine"),("Ultrapreciso","+ sanitizer JSON")]
    n=len(data)
    for i,(t,sub) in enumerate(data):
        x = x0+60 + i*((x1-x0-120)/(n-1))
        r = 7+i*2.4
        s.append(f'<circle cx="{x}" cy="{y}" r="{r+7}" fill="{c["bg"]}"/>')
        s.append(f'<circle cx="{x}" cy="{y}" r="{r}" fill="{c["ink"]}" fill-opacity="{0.35+i*0.2:.2f}"/>')
        s.append(f'<text x="{x}" y="{y-40}" text-anchor="middle" font-family="{SANS}" font-size="20" font-weight="600" fill="{c["ink"]}">{t}</text>')
        s.append(f'<text x="{x}" y="{y+44}" text-anchor="middle" font-family="{MONO}" font-size="12.5" fill="{c["muted"]}">{esc(sub)}</text>')
    s.append('</svg>')
    open(f"{OUT}/{name}", "w").write("\n".join(s))

for k,c in T.items():
    hero(f"hero-{k}.svg", c); flow(f"flow-{k}.svg", c); modes(f"modes-{k}.svg", c)
