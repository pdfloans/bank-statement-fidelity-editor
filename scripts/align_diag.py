#!/usr/bin/env python3
"""Round-trip fidelity regression harness (self-contained).

For each numeric span in the sample statements, round-trips an edit to its
ORIGINAL value through `replace_text_in_rect`, then renders the edited region
at 600 DPI and measures the localized L1 diff against the untouched original
under a best-(dx,dy)-shift alignment.

  * `zero`    : diff with no shift  (placement + shape fidelity)
  * `aligned` : diff after best shift (isolates pure shape fidelity)
  * `@(dx,dy)`: the recovered shift (drift in 600-DPI px)

A faithful edit has low `zero`, `aligned` ~= `zero`, and shift ~ (0,0).
This needs no baseline copy — it validates the CURRENT emit path. Run after
any change to the Python emit/placement code.

    py scripts/align_diag.py                       # all sample PDFs
    py scripts/align_diag.py "AU Bank Statements/Westpac ChoiceBasic.pdf"
"""
import os
import sys

import numpy as np
import pymupdf
import pymupdf.pro

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))
import pymupdf_pro_integration as P  # noqa: E402

KEY = P.PYMUPDF_PRO_KEY


def gray(pdf, pno, rect, dpi=600, pad=8.0):
    pymupdf.pro.unlock(KEY)
    d = pymupdf.open(pdf)
    pg = d[pno]
    clip = pymupdf.Rect(rect[0] - pad, rect[1] - pad, rect[2] + pad, rect[3] + pad)
    pix = pg.get_pixmap(clip=clip, dpi=dpi, alpha=False)
    a = np.frombuffer(pix.samples, dtype=np.uint8).reshape(
        pix.height, pix.width, pix.n
    )[:, :, :3].mean(axis=2)
    d.close()
    return a


def best_shift(o, e, rng=6):
    h = min(o.shape[0], e.shape[0])
    w = min(o.shape[1], e.shape[1])
    o = o[:h, :w]
    e = e[:h, :w]
    best = (1e9, 0, 0)
    m = slice(rng, h - rng), slice(rng, w - rng)
    for dy in range(-rng, rng + 1):
        for dx in range(-rng, rng + 1):
            es = np.roll(np.roll(e, dy, axis=0), dx, axis=1)
            d = float(np.abs(o[m] - es[m]).mean()) / 255.0
            if d < best[0]:
                best = (d, dx, dy)
    d0 = float(np.abs(o[m] - e[m]).mean()) / 255.0
    return d0, best


def _is_std14(name):
    n = (name or "").lower()
    if "+" in n:
        n = n.split("+", 1)[1]
    return n in P._STANDARD_14_FONTS


def spans(pdf, want_subset, limit=4):
    pymupdf.pro.unlock(KEY)
    d = pymupdf.open(pdf)
    out = []
    for pno, page in enumerate(d):
        for b in page.get_text("dict").get("blocks", []):
            for ln in b.get("lines", []):
                for s in ln.get("spans", []):
                    t = (s.get("text") or "").strip()
                    if P._looks_numeric(t) and sum(c.isdigit() for c in t) >= 2:
                        if (not _is_std14(s.get("font"))) == want_subset:
                            out.append((pno, list(s["bbox"]), t, s.get("font")))
                            if len(out) >= limit:
                                d.close()
                                return out
    d.close()
    return out


def run(pdf):
    os.makedirs("output", exist_ok=True)
    print(f"\n=== {os.path.basename(pdf)} ===")
    for ws in (True, False):
        kind = "subset" if ws else "std14"
        for pno, rect, txt, font in spans(pdf, ws):
            try:
                P.replace_text_in_rect(pdf, "output/al_new.pdf", pno, rect, txt)
                o = gray(pdf, pno, rect)
                d0, (dbest, dx, dy) = best_shift(o, gray("output/al_new.pdf", pno, rect))
                verdict = "OK" if d0 < 0.06 else ("review" if d0 < 0.12 else "HIGH")
                print(f"  [{kind}] {font!r} {txt!r}")
                print(f"      zero={d0:.4f} aligned={dbest:.4f} @({dx},{dy})  [{verdict}]")
            except Exception as e:
                print(f"  [{kind}] {font!r} {txt!r}  skip: {str(e)[:50]}")


if __name__ == "__main__":
    pdfs = sys.argv[1:] or [
        os.path.join("AU Bank Statements", f)
        for f in os.listdir("AU Bank Statements")
        if f.lower().endswith(".pdf")
    ]
    for p in pdfs:
        try:
            run(p)
        except Exception as e:
            print(f"{p}: {e}")
