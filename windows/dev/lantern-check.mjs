// Checks the lantern port in a real browser engine (headless Edge, the same
// Chromium that WebView2 runs), with no input to the desktop:
//
//   node dev/lantern-check.mjs [outDir]
//
// 1. Fidelity: the ported component and the approved design
//    (docs/brand/glim-mascot.html) render the same pixels (to within ±2/255
//    of rasterisation rounding, every exception listed), for every state,
//    full and compact, with animations frozen at the same instants (state
//    transitions finished first, so each frame is the state at rest).
// 2. Reduced motion: with prefers-reduced-motion emulated through the DevTools
//    protocol (not the OS setting), no lantern animation runs in any state;
//    without it, every state animates.
// 3. Notch-size frames: the compact sprite at the size the notch draws it,
//    frozen on the blink and glance keyframes, saved to outDir (actual size
//    and 8x) so they can be judged by eye.
//
// Exits non-zero if 1 or 2 fails. Needs Microsoft Edge (present on Windows and
// on GitHub's windows-latest runners). Loads local files only.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { portSvg } from "../scripts/port-lantern.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const DESIGN = resolve(here, "../../docs/brand/glim-mascot.html");
const CSS = resolve(here, "../src/mascot/lantern.css");
const STATES = ["idle", "listen", "think", "suggest", "record", "paused", "delegate"];
/** Largest per-channel difference (of 255) still counted as the same pixel.
 * Chromium's blur under a running brightness filter (s-idle) isn't bit-stable
 * from frame to frame: renders of the very same page differ by 1 now and then. */
const ROUNDING = 2;
/** Instants (ms into every animation) at which both renders are compared. */
const FREEZE_AT = [0, 450, 1100, 2000, 3300];
/** The notch's compact sprite: 1.2 × the diameter 20 over the 99 drawn
 * viewBox units (LANTERN_HEIGHT_PER_DIAMETER in src/mascot/lantern.ts). */
const NOTCH_UNIT = (20 * 1.2) / 99;
const NOTCH_W = 100 * NOTCH_UNIT;
const NOTCH_H = 124 * NOTCH_UNIT;

const outDir = resolve(process.argv[2] ?? join(tmpdir(), "glim-lantern-check"));
mkdirSync(outDir, { recursive: true });

// ── The page under test: the port, as the app instantiates it ────────────────

function portPage(dir) {
  // The markup exactly as lantern-svg.ts holds it, with one instance's ids.
  const src = portSvg(readFileSync(DESIGN, "utf8"));
  const markup = JSON.parse(src.match(/LANTERN_MARKUP = (".*");/)[1]).replaceAll("{ID}", "p1");
  const viewBox = JSON.parse(src.match(/LANTERN_VIEWBOX = (".*");/)[1]);
  const html = `<!doctype html><meta charset="utf-8"><style>${readFileSync(CSS, "utf8")}</style>
<body style="margin:0"><svg id="glim" class="lantern s-listen" viewBox="${viewBox}" width="240" height="298" aria-hidden="true">${markup}</svg>
<script>function setState(s){const el=document.getElementById('glim');el.classList.remove(${STATES.map((s) => `'s-${s}'`).join(",")});el.classList.add(s);}</script>`;
  const path = join(dir, "port.html");
  writeFileSync(path, html);
  return pathToFileURL(path).href;
}

// ── A minimal DevTools-protocol client ───────────────────────────────────────

async function launchEdge(profile) {
  const exe = ["ProgramFiles(x86)", "ProgramFiles"]
    .map((v) => process.env[v] && join(process.env[v], "Microsoft", "Edge", "Application", "msedge.exe"))
    .find((p) => p && existsSync(p));
  if (!exe) throw new Error("Microsoft Edge not found");
  const proc = spawn(exe, [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--disable-background-networking", "--disable-component-update", "--no-first-run",
    "--force-color-profile=srgb", "about:blank",
  ], { stdio: "ignore" });
  const portFile = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 100 && !existsSync(portFile); i++) await new Promise((r) => setTimeout(r, 100));
  const port = readFileSync(portFile, "utf8").split("\n")[0];
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, e) => { ws.onopen = r; ws.onerror = e; });
  let id = 0;
  const pending = new Map();
  const waiters = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { ok, err } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? err(new Error(JSON.stringify(msg.error))) : ok(msg.result);
    } else if (msg.method) {
      for (const w of waiters.filter((w) => w.method === msg.method)) {
        waiters.splice(waiters.indexOf(w), 1);
        w.ok(msg.params);
      }
    }
  };
  const send = (method, params = {}) =>
    new Promise((ok, err) => {
      pending.set(++id, { ok, err });
      ws.send(JSON.stringify({ id, method, params }));
    });
  const once = (method) => new Promise((ok) => waiters.push({ method, ok }));
  return { send, once, close: () => { ws.close(); proc.kill(); } };
}

async function main() {
  const work = mkdtempSync(join(tmpdir(), "glim-lantern-"));
  const cdp = await launchEdge(join(work, "profile"));
  const failures = [];
  try {
    const { send, once } = cdp;
    await send("Page.enable");
    await send("Emulation.setDeviceMetricsOverride", { width: 600, height: 700, deviceScaleFactor: 1, mobile: false });
    const evaluate = async (expression) =>
      (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result.value;
    const open = async (url) => {
      const loaded = once("Page.loadEventFired");
      await send("Page.navigate", { url });
      await loaded;
      // A plain backdrop and the same place on both pages, so the page around
      // the <svg> can't change a pixel.
      await evaluate(`(()=>{const s=document.createElement('style');s.textContent='body{background:#0c111c !important}.controls{display:none}#glim{position:fixed;left:120px;top:100px}';document.head.append(s);})()`);
    };
    // `settle`: wait out the state change's transitions (longest .35s) before
    // freezing, so a slow machine can't start one after it was finished.
    const setUp = (state, compact, at, settle = false) =>
      evaluate(`(async()=>{setState('s-${state}');const el=document.getElementById('glim');el.classList.toggle('compact',${compact});
        await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
        if(${settle})await new Promise(r=>setTimeout(r,600));
        for(const a of document.getAnimations()){if(a.constructor.name==='CSSTransition'){a.finish();}else{a.pause();a.currentTime=${at};}}
        await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
        const b=el.getBoundingClientRect();return {x:b.x,y:b.y,width:b.width,height:b.height,animations:document.getAnimations().length};})()`);
    const shot = async (clip) =>
      (await send("Page.captureScreenshot", { format: "png", clip: { ...clip, scale: 1 } })).data;

    // 1. Fidelity.
    const designUrl = pathToFileURL(DESIGN).href;
    const portUrl = portPage(work);
    const renders = { design: {}, port: {} };
    for (const [name, url] of [["design", designUrl], ["port", portUrl]]) {
      for (const state of STATES) {
        for (const compact of [false, true]) {
          // A fresh page for each state and size: the same history on both pages.
          await open(url);
          for (const at of FREEZE_AT) {
            const r = await setUp(state, compact, at, at === FREEZE_AT[0]);
            // The glow and the aura reach past the viewBox: capture around it.
            renders[name][`${state}/${compact}/${at}`] = await shot({ x: r.x - 60, y: r.y - 40, width: r.width + 120, height: r.height + 80 });
          }
        }
      }
    }
    // Byte-identical PNGs are identical pixels; otherwise count what differs.
    const diff = (a, b) =>
      evaluate(`(async()=>{const load=s=>new Promise(r=>{const i=new Image();i.onload=()=>r(i);i.src='data:image/png;base64,'+s;});
        const [x,y]=await Promise.all([load(${JSON.stringify(a)}),load(${JSON.stringify(b)})]);
        if(x.width!==y.width||x.height!==y.height)return {pixels:-1,max:255};
        const px=i=>{const c=document.createElement('canvas');c.width=i.width;c.height=i.height;const g=c.getContext('2d');g.drawImage(i,0,0);return g.getImageData(0,0,i.width,i.height).data;};
        const p=px(x),q=px(y);let n=0,m=0;for(let i=0;i<p.length;i+=4){let d=0;for(let k=0;k<4;k++)d=Math.max(d,Math.abs(p[i+k]-q[i+k]));if(d){n++;m=Math.max(m,d);}}
        return {pixels:n,max:m,total:p.length/4};})()`);
    let compared = 0;
    let exact = 0;
    const near = [];
    for (const key of Object.keys(renders.design)) {
      compared++;
      if (renders.design[key] === renders.port[key]) {
        exact++;
        continue;
      }
      const d = await diff(renders.design[key], renders.port[key]);
      near.push(`${key}: ${d.pixels} of ${d.total} pixels differ, by at most ${d.max}/255`);
      if (d.pixels < 0 || d.max > ROUNDING) {
        const tag = key.replaceAll("/", "-");
        writeFileSync(join(outDir, `mismatch-${tag}-design.png`), Buffer.from(renders.design[key], "base64"));
        writeFileSync(join(outDir, `mismatch-${tag}-port.png`), Buffer.from(renders.port[key], "base64"));
        failures.push(`pixels differ: ${key}`);
      }
    }
    console.log(`fidelity: ${exact}/${compared} renders byte-identical to the design`);
    for (const n of near) console.log(`  ${n}`);

    // 2. Reduced motion, emulated.
    await open(portUrl);
    for (const reduce of [false, true]) {
      await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: reduce ? "reduce" : "no-preference" }] });
      const matches = await evaluate("matchMedia('(prefers-reduced-motion: reduce)').matches");
      if (matches !== reduce) failures.push(`media emulation not applied (reduce=${reduce})`);
      for (const state of STATES) {
        for (const compact of [false, true]) {
          const running = await evaluate(`(async()=>{setState('s-${state}');document.getElementById('glim').classList.toggle('compact',${compact});
            await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
            return document.getAnimations().filter(a=>a.constructor.name==='CSSAnimation'&&a.playState==='running').map(a=>a.animationName);})()`);
          if (reduce && running.length) failures.push(`reduced motion: ${state}${compact ? " compact" : ""} still runs ${running.join(", ")}`);
          if (!reduce && !running.length) failures.push(`no animation at all in ${state}${compact ? " compact" : ""}`);
        }
      }
      console.log(`reduced motion ${reduce ? "on " : "off"}: checked ${STATES.length * 2} state/size pairs`);
    }
    await send("Emulation.setEmulatedMedia", { features: [] });

    // 3. Notch-size frames of the blink and glance keyframes.
    // As the island draws it: the notch detail (no aura, no twinkles) on the
    // island's black.
    await evaluate(`(()=>{const el=document.getElementById('glim');el.setAttribute('width','${NOTCH_W}');el.setAttribute('height','${NOTCH_H}');
      el.querySelector('.aura').remove();el.querySelector('.twinkles').remove();document.body.style.setProperty('background','#000','important');})()`);
    const frames = [
      ["listen-rest", "listen", 0], ["listen-glance-left", "listen", 2400], ["listen-glance-right", "listen", 3400],
      ["listen-blink", "listen", 4500], ["idle-doze", "idle", 1800], ["think-ponder", "think", 1440],
      ["think-blink", "think", 1600], ["suggest-blink", "suggest", 2040], ["record-blink", "record", 5850],
    ];
    for (const [name, state, at] of frames) {
      const r = await setUp(state, true, at, true);
      const clip = { x: r.x - 4, y: r.y - 4, width: r.width + 8, height: r.height + 8 };
      writeFileSync(join(outDir, `notch-${name}.png`), Buffer.from(await shot(clip), "base64"));
      await send("Emulation.setDeviceMetricsOverride", { width: 600, height: 700, deviceScaleFactor: 8, mobile: false });
      writeFileSync(join(outDir, `notch-${name}@8x.png`), Buffer.from(await shot(clip), "base64"));
      await send("Emulation.setDeviceMetricsOverride", { width: 600, height: 700, deviceScaleFactor: 1, mobile: false });
    }
    console.log(`notch-size frames (${NOTCH_W.toFixed(1)}×${NOTCH_H.toFixed(1)} px): ${outDir}`);
  } finally {
    cdp.close();
    await new Promise((r) => setTimeout(r, 500));
    rmSync(work, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
  if (failures.length) {
    console.error(failures.join("\n"));
    process.exit(1);
  }
  console.log("lantern check passed");
}

await main();
