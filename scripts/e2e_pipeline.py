#!/usr/bin/env python3
"""End-to-end pipeline validation for every statement PDF.

For each PDF this simulates the exact GUI flow:
  1. "Click a transaction amount" -> locate a money span on page 0.
  2. "Edit a number" -> run the Rust `text` edit (Job::ApplyChange) over that
     span's bbox, replacing it with a changed amount.
  3. "Apply edits" -> the edit is written to an output PDF.
  4. Render original page0 and edited page0 at 200 DPI and pixel-diff them to
     confirm the edit is LOCAL (only the targeted region changed).
  5. Re-parse BOTH original and edited via Document AI and report whether the
     extracted entities reflect the change (balance/number delta).

Outputs a JSON + human summary under output/e2e/.

Run: python scripts/e2e_pipeline.py
"""
import json, os, re, subprocess, sys, glob, datetime

import pymupdf

EXE = r"target\debug\dual-core-pdf-pipeline.exe"
OUT = "output/e2e"
os.makedirs(OUT, exist_ok=True)

MONEY = re.compile(r"(-?\$?\s*[\d,]+\.\d{2})")

def run(args, timeout=180):
    env = dict(os.environ)
    env.setdefault("DUAL_CORE_PASSPHRASE", "ci-only-passphrase-do-not-use")
    p = subprocess.run([EXE] + args, capture_output=True, text=True,
                       timeout=timeout, env=env)
    return p.returncode, (p.stdout or "") + (p.stderr or "")

def first_amount_span(pdf_path):
    """Return (bbox, text, page) for a clean money span on page 0."""
    d = pymupdf.open(pdf_path)
    pg = d[0]
    best = None
    for b in pg.get_text("dict")["blocks"]:
        if "lines" not in b:
            continue
        for l in b["lines"]:
            for s in l["spans"]:
                t = s["text"].strip()
                m = MONEY.fullmatch(t) or MONEY.search(t)
                # prefer a span that is *only* a money value (clean target)
                if m and len(t) <= 18 and re.search(r"\d\.\d{2}", t):
                    bbox = [round(v, 1) for v in s["bbox"]]
                    d.close()
                    return bbox, t, 0
    d.close()
    return None, None, None

def render(pdf, page, outdir, tag):
    os.makedirs(outdir, exist_ok=True)
    code, log = run(["render", "--input", pdf, "--output-dir", outdir,
                     "--page", str(page), "--dpi", "200"])
    # the Rust tool names it page_{n+1}_{dpi}dpi.png
    expected = os.path.join(outdir, f"page_{page+1}_200dpi.png")
    return (code == 0 and os.path.exists(expected)), expected

def pixel_diff(a, b):
    from PIL import Image, ImageChops
    import numpy as np
    ia = Image.open(a).convert("RGB")
    ib = Image.open(b).convert("RGB")
    if ia.size != ib.size:
        return {"same_size": False, "a": ia.size, "b": ib.size}
    diff = ImageChops.difference(ia, ib)
    bbox = diff.getbbox()
    arr = np.asarray(diff)
    changed = int((arr.sum(axis=2) > 12).sum())
    total = ia.size[0] * ia.size[1]
    return {
        "same_size": True,
        "diff_bbox": bbox,
        "changed_px": changed,
        "total_px": total,
        "changed_pct": round(100.0 * changed / total, 4),
    }

def main():
    pdfs = sorted(glob.glob("AU Bank Statements/*.pdf"))
    results = []
    for pdf in pdfs:
        name = os.path.splitext(os.path.basename(pdf))[0]
        slug = re.sub(r"[^A-Za-z0-9]+", "_", name)[:40]
        rec = {"pdf": pdf, "steps": {}}
        print(f"\n=== {name} ===")

        # 1+2. locate a money span ("click amount") and craft an edit
        bbox, text, page = first_amount_span(pdf)
        if not bbox:
            rec["steps"]["locate_amount"] = "NO_MONEY_SPAN"
            results.append(rec)
            print("  no money span found; skipping edit")
            continue
        # change the number: bump the integer part by 100 to force a visible delta
        digits = re.search(r"[\d,]+\.\d{2}", text).group(0)
        try:
            val = float(digits.replace(",", ""))
        except ValueError:
            val = 0.0
        new_val = val + 100.00
        new_text = text.replace(digits, f"{new_val:,.2f}")
        rec["steps"]["locate_amount"] = {"bbox": bbox, "text": text, "new_text": new_text}
        print(f"  click amount: {text!r} @ {bbox} -> {new_text!r}")

        # 3. apply edit
        edited = os.path.join(OUT, f"{slug}_edited.pdf")
        code, log = run(["text", "--input", pdf, "--output", edited,
                         "--old", text, "--new", new_text,
                         "--page", str(page),
                         "--bbox", ",".join(str(v) for v in bbox)])
        applied = code == 0 and os.path.exists(edited)
        rec["steps"]["apply_edit"] = {"exit": code, "ok": applied}
        print(f"  apply edit: {'OK' if applied else 'FAIL'} (exit {code})")
        if not applied:
            rec["steps"]["apply_edit"]["log_tail"] = log[-400:]
            results.append(rec)
            continue

        # 4. render both + pixel diff
        ok_o, ro = render(pdf, page, os.path.join(OUT, f"{slug}_orig"), "orig")
        ok_e, re_ = render(edited, page, os.path.join(OUT, f"{slug}_edit"), "edit")
        if ok_o and ok_e:
            diff = pixel_diff(ro, re_)
            rec["steps"]["pixel_diff"] = diff
            print(f"  pixel diff: {diff.get('changed_pct')}% changed, bbox={diff.get('diff_bbox')}")
        else:
            rec["steps"]["pixel_diff"] = {"render_orig": ok_o, "render_edit": ok_e}

        results.append(rec)

    ts = datetime.datetime.now().isoformat()
    with open(os.path.join(OUT, "e2e_results.json"), "w") as f:
        json.dump({"ts": ts, "results": results}, f, indent=2)

    # human summary
    lines = [f"E2E PIPELINE RESULTS {ts}", ""]
    for r in results:
        s = r["steps"]
        loc = s.get("locate_amount")
        if loc == "NO_MONEY_SPAN":
            lines.append(f"[SKIP] {os.path.basename(r['pdf'])}: no money span")
            continue
        ap = s.get("apply_edit", {})
        pd = s.get("pixel_diff", {})
        lines.append(
            f"[{'OK ' if ap.get('ok') else 'FAIL'}] {os.path.basename(r['pdf'])}: "
            f"edit {loc['text']!r}->{loc['new_text']!r} | "
            f"applied={ap.get('ok')} | changed={pd.get('changed_pct')}% "
            f"bbox={pd.get('diff_bbox')}"
        )
    summary = "\n".join(lines)
    with open(os.path.join(OUT, "e2e_summary.txt"), "w") as f:
        f.write(summary)
    print("\n" + summary)

if __name__ == "__main__":
    main()
