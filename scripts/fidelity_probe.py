#!/usr/bin/env python3
"""Fidelity probe harness.

Picks a numeric span from a real bank statement, edits it in place via
`replace_text_in_rect`, then renders the edited region at high DPI on both the
original and edited PDFs and reports a localized diff. Used to validate the
Stage A-G fidelity work without needing the full Rust pipeline.

Usage:
    py scripts/fidelity_probe.py "AU Bank Statements/Westpac ChoiceBasic.pdf"
"""
import os
import sys
import json

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

import pymupdf  # noqa: E402
import pymupdf.pro  # noqa: E402
import pymupdf_pro_integration as P  # noqa: E402


def _first_numeric_span(doc):
    """Return (page_num, span_dict) for the first amount-looking span."""
    for pno, page in enumerate(doc):
        for block in page.get_text("dict").get("blocks", []):
            for line in block.get("lines", []):
                for span in line.get("spans", []):
                    txt = (span.get("text") or "").strip()
                    if P._looks_numeric(txt) and sum(c.isdigit() for c in txt) >= 3:
                        return pno, span
    return None, None


def _render_region(pdf_path, page_num, rect, dpi=600, pad=4.0):
    pymupdf.pro.unlock(P.PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    page = doc[page_num]
    clip = pymupdf.Rect(rect[0] - pad, rect[1] - pad, rect[2] + pad, rect[3] + pad)
    pix = page.get_pixmap(clip=clip, dpi=dpi, alpha=False)
    doc.close()
    return pix


def _diff_score(pix_a, pix_b):
    import numpy as np
    h = min(pix_a.height, pix_b.height)
    w = min(pix_a.width, pix_b.width)
    a = np.frombuffer(pix_a.samples, dtype=np.uint8).reshape(pix_a.height, pix_a.width, pix_a.n)
    b = np.frombuffer(pix_b.samples, dtype=np.uint8).reshape(pix_b.height, pix_b.width, pix_b.n)
    a = a[:h, :w, :3].astype(np.int16)
    b = b[:h, :w, :3].astype(np.int16)
    return float(np.abs(a - b).mean()) / 255.0


def main():
    pdf = sys.argv[1] if len(sys.argv) > 1 else "AU Bank Statements/Westpac ChoiceBasic.pdf"
    new_text = sys.argv[2] if len(sys.argv) > 2 else None

    pymupdf.pro.unlock(P.PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf)
    pno, span = _first_numeric_span(doc)
    if span is None:
        print("no numeric span found")
        return 1
    bbox = list(span["bbox"])
    orig_text = span["text"]
    doc.close()

    if new_text is None:
        # Mutate one digit so the shape changes but width is similar.
        digits = [c for c in orig_text if c.isdigit()]
        new_text = orig_text.replace(digits[-1], "8" if digits[-1] != "8" else "3", 1)

    print(f"page={pno} bbox={bbox}")
    print(f"font={span.get('font')} size={span.get('size')}")
    print(f"orig={orig_text!r} -> new={new_text!r}")

    out_pdf = os.path.join("output", "fidelity_probe_out.pdf")
    os.makedirs("output", exist_ok=True)
    result = P.replace_text_in_rect(
        pdf_path=pdf,
        output_path=out_pdf,
        page_num=pno,
        rect=bbox,
        new_text=new_text,
    )
    print("method:", json.dumps(result))

    # Render the SAME text edit back to the original value to measure
    # round-trip fidelity (edit orig->orig should be near-zero diff).
    rt_pdf = os.path.join("output", "fidelity_probe_roundtrip.pdf")
    rt = P.replace_text_in_rect(
        pdf_path=pdf, output_path=rt_pdf, page_num=pno, rect=bbox, new_text=orig_text
    )
    print("roundtrip method:", json.dumps(rt))

    a = _render_region(pdf, pno, bbox)
    b = _render_region(rt_pdf, pno, bbox)
    score = _diff_score(a, b)
    print(f"ROUNDTRIP localized diff @600dpi (orig vs edited-to-same): {score:.5f}")
    print("  (lower is better; this isolates font/spacing fidelity)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
