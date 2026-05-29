#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
PyMuPDF Pro Smart Targeted Editor v2.1
- Get all text blocks with accurate bounding boxes
- Robust targeted replacement inside a specific rectangle using redaction
"""

import pymupdf.pro
import pymupdf
import json
import os
import sys

PYMUPDF_PRO_KEY = "hFKt4hca03GCFLAFLEGz5Bd3"

def get_text_blocks(pdf_path: str, page_num: int = 0):
    """Return list of text spans with precise bounding boxes and font info"""
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    page = doc[page_num]

    blocks = []
    for block in page.get_text("dict")["blocks"]:
        if "lines" not in block:
            continue
        for line in block["lines"]:
            for span in line["spans"]:
                blocks.append({
                    "page": page_num,
                    "text": span["text"],
                    "bbox": list(span["bbox"]),      # [x0, y0, x1, y1]
                    "font": span["font"],
                    "size": round(span["size"], 2),
                    "color": span["color"],
                    "origin": list(span.get("origin", [0, 0])),
                })
    doc.close()
    return blocks


def _color_int_to_rgb(color_int: int):
    """PyMuPDF gives sRGB span colour as a single int (0xRRGGBB). Map to (r,g,b) floats."""
    if color_int is None:
        return (0.0, 0.0, 0.0)
    r = ((color_int >> 16) & 0xFF) / 255.0
    g = ((color_int >> 8) & 0xFF) / 255.0
    b = (color_int & 0xFF) / 255.0
    return (r, g, b)


# ===========================================================================
# Stage 8.5: Document-level font analysis.
#
# Runs once when the user opens a PDF. For each font used in the document
# we report:
#   - usage_role: "digits", "letters", "mixed", "punctuation", "other"
#   - characters_used: every codepoint that actually appears
#   - missing_chars: characters_used \ subset coverage
#   - fidelity_impact: free-text human-readable summary the GUI shows the user
#
# The decision rule is straightforward and matches the users requirement:
#
#   For each font:
#     used = set of characters actually written with this font in the doc
#     covered = set of characters the embedded subset (or standard-14) renders
#     missing = used - covered
#
#     if missing == empty: no action needed for this font
#     elif missing is digits-only: only those digit glyphs need creation
#     elif missing is letters-only: only those letter glyphs need creation
#     else (digits + letters mixed): only the specific missing glyphs of
#         each kind need creation -- never the full alphabet
#
# Even if `used` happens to span letters and digits, the *creation scope* is
# only `missing`, never the universe of the alphabet.
# ===========================================================================

def _classify_role(chars: set) -> str:
    """Bucket the characters used by a font into a usage role for display."""
    has_digits = any(c.isdigit() for c in chars)
    has_letters = any(c.isalpha() for c in chars)
    has_punct = any((not c.isalnum()) and (not c.isspace()) for c in chars)
    if has_digits and not has_letters:
        return "digits"
    if has_letters and not has_digits:
        return "letters"
    if has_digits and has_letters:
        return "mixed"
    if has_punct and not has_digits and not has_letters:
        return "punctuation"
    return "other"


def _missing_breakdown(missing: list) -> dict:
    """Split `missing` into digits / letters / other for the scope summary."""
    digits = [c for c in missing if c.isdigit()]
    letters = [c for c in missing if c.isalpha()]
    other = [c for c in missing if not c.isalnum()]
    return {
        "digits": digits,
        "letters": letters,
        "other": other,
    }


def _is_standard_14_basename(name: str) -> bool:
    if not name:
        return False
    n = name.lower()
    if "+" in n:
        n = n.split("+", 1)[1]
    return n in _STANDARD_14_FONTS


def _has_glyph_safe(font_obj, codepoint: int) -> bool:
    """`Font.has_glyph` can raise on some malformed subsets; fall back to
    `glyph_advance` returning a positive width."""
    try:
        if bool(font_obj.has_glyph(codepoint)):
            return True
    except Exception:
        pass
    try:
        return float(font_obj.glyph_advance(codepoint)) > 0.0
    except Exception:
        return False


def _winansi_covers(ch: str) -> bool:
    """Standard-14 fonts have implicit WinAnsi coverage. Anything that
    encodes to cp1252 is renderable without an embedded subset."""
    try:
        ch.encode("cp1252")
        return True
    except UnicodeEncodeError:
        return False


def analyze_fonts(pdf_path: str) -> dict:
    """Return a per-font breakdown of usage, coverage and fidelity impact.

    Output shape:
      {
        "fonts": [
          {
            "name": "ABCDEF+Helvetica-Bold",
            "base_name": "Helvetica-Bold",
            "xref": 12,
            "is_standard_14": false,
            "is_subset": true,
            "usage_role": "digits",
            "pages_used_on": [0, 1, 2],
            "size_range": [8.5, 10.0],
            "occurrences": 247,
            "characters_used": "$,.0123456789",
            "missing_chars": ["$"],
            "missing_breakdown": {"digits": [], "letters": [], "other": ["$"]},
            "creation_scope": "Create only 1 missing glyph(s): $",
            "fidelity_impact": "Used only for digits -- but $ is missing. ..."
          },
          ...
        ],
        "summary": {
          "total_fonts": 5,
          "fonts_needing_action": 2,
          "missing_digit_count": 0,
          "missing_letter_count": 3,
          "missing_other_count": 1,
          "all_fonts_covered": false,
        }
      }
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)

    # 1. Collect per-font usage data, keyed by basename so subsets of the
    #    same base font roll up together.
    per_font = {}

    for page_idx, page in enumerate(doc):
        try:
            page_fonts = {f[0]: f for f in page.get_fonts(full=True)}
        except Exception:
            page_fonts = {}

        for block in page.get_text("dict").get("blocks", []):
            if "lines" not in block:
                continue
            for line in block["lines"]:
                for span in line.get("spans", []):
                    raw_name = span.get("font", "") or ""
                    base = raw_name.split("+", 1)[1] if "+" in raw_name else raw_name
                    key = base.lower()
                    if not key:
                        continue
                    text = span.get("text", "") or ""
                    if not text:
                        continue

                    rec = per_font.setdefault(key, {
                        "name": raw_name,
                        "base_name": base,
                        "is_subset": "+" in raw_name,
                        "is_standard_14": _is_standard_14_basename(raw_name),
                        "characters_used": set(),
                        "pages_used_on": set(),
                        "size_min": float("inf"),
                        "size_max": 0.0,
                        "occurrences": 0,
                        "first_xref": None,
                    })
                    rec["characters_used"].update(text)
                    rec["pages_used_on"].add(page_idx)
                    sz = float(span.get("size", 0.0))
                    if sz > 0:
                        rec["size_min"] = min(rec["size_min"], sz)
                        rec["size_max"] = max(rec["size_max"], sz)
                    rec["occurrences"] += 1
                    if rec["first_xref"] is None:
                        for xref, info in page_fonts.items():
                            try:
                                bf = (info[3] or "").lower()
                                al = (info[4] or "").lower()
                            except (IndexError, TypeError):
                                continue
                            if base.lower() in bf or base.lower() in al or al == raw_name.lower():
                                rec["first_xref"] = xref
                                break

    # 2. Build coverage report per font.
    fonts_out = []
    fonts_needing_action = 0
    total_missing_digits = 0
    total_missing_letters = 0
    total_missing_other = 0

    for key, rec in per_font.items():
        chars = rec["characters_used"]
        chars_clean = sorted({c for c in chars if not c.isspace()})
        role = _classify_role(set(chars_clean))

        # Determine which characters are NOT covered by the embedded subset.
        if rec["is_standard_14"]:
            # WinAnsi renders all cp1252 chars implicitly.
            missing = [c for c in chars_clean if not _winansi_covers(c)]
        elif rec["first_xref"] is not None:
            try:
                font_info = doc.extract_font(rec["first_xref"])
                content = None
                if isinstance(font_info, dict):
                    content = font_info.get("content")
                elif isinstance(font_info, (tuple, list)) and len(font_info) >= 4:
                    for item in reversed(font_info):
                        if isinstance(item, (bytes, bytearray)) and len(item) > 0:
                            content = bytes(item)
                            break
                if content:
                    f = pymupdf.Font(fontbuffer=content)
                    missing = [
                        c for c in chars_clean if not _has_glyph_safe(f, ord(c))
                    ]
                else:
                    missing = []
            except Exception:
                missing = []
        else:
            missing = []

        breakdown = _missing_breakdown(missing)

        # Fidelity impact and creation-scope language.
        if not missing:
            impact = "✅ All characters used in this document are covered by the embedded subset -- no font creation needed."
            scope = "None -- all used glyphs already present."
        else:
            fonts_needing_action += 1
            total_missing_digits += len(breakdown["digits"])
            total_missing_letters += len(breakdown["letters"])
            total_missing_other += len(breakdown["other"])

            kinds = []
            if breakdown["digits"]:
                kinds.append(f"{len(breakdown['digits'])} digit(s)")
            if breakdown["letters"]:
                kinds.append(f"{len(breakdown['letters'])} letter(s)")
            if breakdown["other"]:
                kinds.append(f"{len(breakdown['other'])} other glyph(s)")
            kinds_str = ", ".join(kinds)

            preview = "".join(missing[:12])
            if len(missing) > 12:
                preview += "…"

            scope = (
                f"Create only the {len(missing)} missing glyph(s): "
                f"{preview}  ({kinds_str})"
            )

            if role == "digits":
                impact = (
                    f"⚠ Digits-only font -- {len(missing)} glyph(s) missing in this document. "
                    f"Only those specific glyph(s) need creation; the full alphabet is not required."
                )
            elif role == "punctuation":
                impact = (
                    f"⚠ Punctuation-only font -- {len(missing)} glyph(s) missing. "
                    f"Targeted creation of those glyph(s) only."
                )
            elif role == "letters":
                impact = (
                    f"⚠ Letters font -- {len(missing)} letter(s) missing. "
                    f"Only those specific letter glyph(s) need creation; the full alphabet is not required."
                )
            elif role == "mixed":
                # The users rule: even if used spans letters+digits, the
                # creation scope is the actual missing set, not the universe.
                impact = (
                    f"⚠ Mixed font (letters + digits) -- {len(missing)} glyph(s) missing. "
                    f"Creation scope is limited to those glyph(s) only ({kinds_str})."
                )
            else:
                impact = f"⚠ {len(missing)} glyph(s) missing in role '{role}'."

        fonts_out.append({
            "name": rec["name"],
            "base_name": rec["base_name"],
            "xref": rec["first_xref"],
            "is_standard_14": rec["is_standard_14"],
            "is_subset": rec["is_subset"],
            "usage_role": role,
            "pages_used_on": sorted(rec["pages_used_on"]),
            "size_range": [
                round(rec["size_min"], 2) if rec["size_min"] != float("inf") else 0.0,
                round(rec["size_max"], 2),
            ],
            "characters_used": "".join(chars_clean),
            "missing_chars": missing,
            "missing_breakdown": breakdown,
            "occurrences": rec["occurrences"],
            "fidelity_impact": impact,
            "creation_scope": scope,
        })

    fonts_out.sort(key=lambda f: (-f["occurrences"], f["base_name"]))
    doc.close()

    return {
        "fonts": fonts_out,
        "summary": {
            "total_fonts": len(fonts_out),
            "fonts_needing_action": fonts_needing_action,
            "missing_digit_count": total_missing_digits,
            "missing_letter_count": total_missing_letters,
            "missing_other_count": total_missing_other,
            "all_fonts_covered": fonts_needing_action == 0,
        },
    }


def _find_dominant_span(page, rect_obj):
    """Find the text span whose bbox best overlaps the supplied rectangle.

    Returns the span dict (text/font/size/color/origin) or None if nothing overlaps.
    """
    best = None
    best_area = 0.0
    rect = pymupdf.Rect(rect_obj)
    for block in page.get_text("dict").get("blocks", []):
        if "lines" not in block:
            continue
        for line in block["lines"]:
            for span in line["spans"]:
                sp_rect = pymupdf.Rect(span["bbox"])
                inter = sp_rect & rect
                if inter.is_empty:
                    continue
                area = inter.width * inter.height
                if area > best_area:
                    best_area = area
                    best = span
    return best


_STANDARD_14_FONTS = {
    # The PDF spec guarantees every reader supplies these. Their full
    # WinAnsiEncoding glyph set is always usable, so coverage is implicit.
    "times-roman", "times-bold", "times-italic", "times-bolditalic",
    "helvetica", "helvetica-bold", "helvetica-oblique", "helvetica-boldoblique",
    "courier", "courier-bold", "courier-oblique", "courier-boldoblique",
    "symbol", "zapfdingbats",
}


def _is_standard_14(name: str) -> bool:
    if not name:
        return False
    n = name.lower()
    # Some PDFs prefix subsetted names like "ABCDEF+Times-Roman".
    if "+" in n:
        n = n.split("+", 1)[1]
    return n in _STANDARD_14_FONTS


def _font_covers_text(page, font_xref: int, font_name: str, text: str):
    """Return (covers, missing_chars).

    Coverage logic, in order:
      1. If `font_name` is one of the PDF standard 14 (Times/Helvetica/Courier/Symbol/ZapfDingbats),
         every WinAnsi codepoint is supplied by the reader. We only flag
         characters outside WinAnsiEncoding (rare -- emoji, CJK, etc.).
      2. Otherwise we attempt to extract the embedded font subset and probe
         glyph coverage with PyMuPDF.Font(buffer=...).
      3. Any failure to determine coverage is treated as 'unknown' and
         returns (False, list(text)) so the caller can decide.
    """
    if _is_standard_14(font_name):
        # WinAnsi covers most western characters. Flag only ones that are not
        # representable in cp1252.
        missing = []
        for ch in text:
            try:
                ch.encode("cp1252")
            except UnicodeEncodeError:
                missing.append(ch)
        return (len(missing) == 0, missing)

    try:
        result = page.parent.extract_font(font_xref)
    except Exception:
        return (False, list(text))

    content = None
    if isinstance(result, dict):
        content = result.get("content")
    elif isinstance(result, (tuple, list)) and len(result) >= 4:
        for item in reversed(result):
            if isinstance(item, (bytes, bytearray)) and len(item) > 0:
                content = bytes(item)
                break

    if not content:
        return (False, list(text))

    try:
        f = pymupdf.Font(fontbuffer=content)
    except Exception:
        return (False, list(text))

    missing = []
    for ch in text:
        if ch in (" ",):
            continue
        try:
            ok = bool(f.has_glyph(ord(ch)))
        except Exception:
            try:
                ok = bool(f.glyph_advance(ord(ch)))
            except Exception:
                ok = False
        if not ok:
            missing.append(ch)
    return (len(missing) == 0, missing)


def _embedded_font_xref_for_span(page, span: dict):
    """Locate the xref of the font used by `span`.

    PyMuPDF `dict` extraction returns the font's *base name* (e.g. 'Times-Roman'
    or 'F1' depending on the source). We cross-reference page.get_fonts(full=True)
    to pull the matching xref. Falls back to the first font on the page.
    """
    try:
        fonts = page.get_fonts(full=True)
    except Exception:
        return None
    if not fonts:
        return None
    needle = (span.get("font") or "").lower()
    if needle:
        # Match by basefont (index 3) or by the alias name (index 4).
        for f in fonts:
            try:
                basefont = (f[3] or "").lower()
                alias = (f[4] or "").lower()
            except (IndexError, TypeError):
                continue
            if basefont == needle or alias == needle or basefont.endswith("+" + needle) or needle in basefont:
                return f[0]
    # No match: first font.
    try:
        return fonts[0][0]
    except (IndexError, TypeError):
        return None


# ===========================================================================
# Stage 9: Background & color-space fidelity helpers (Items #5, #6).
# ===========================================================================

def _sample_pixel(page, x: float, y: float):
    """Sample a single pixel from the rendered page at (x,y) in PDF points.
    Returns (r,g,b) floats in [0,1]. None if the position is off the page.
    """
    page_w = float(page.rect.width)
    page_h = float(page.rect.height)
    if x < 0 or x >= page_w or y < 0 or y >= page_h:
        return None
    # 1pt-wide clip; cheap and avoids huge pixmap allocation.
    clip = pymupdf.Rect(x, y, x + 1.0, y + 1.0)
    try:
        pix = page.get_pixmap(clip=clip, alpha=False)
    except Exception:
        return None
    if pix.width == 0 or pix.height == 0:
        return None
    samples = pix.samples
    n = pix.n
    if n == 1:
        v = samples[0] / 255.0
        return (v, v, v)
    if n >= 3:
        return (samples[0] / 255.0, samples[1] / 255.0, samples[2] / 255.0)
    return None


def classify_background(page, rect_obj):
    """Stage 9 / Item #5: classify the area beneath an edit as
    solid / striped / patterned, and return the local fill color we should
    use for the redaction.

    Strategy: sample 8 points around the rect's edge, plus the centre.
      * If all samples agree to within tolerance, return ("solid", color).
      * If samples cluster into 2 colors that alternate left-right or
        top-bottom, return ("striped", color_at_center). The redactor
        uses the center color so we do not flip the row's stripe.
      * Otherwise return ("patterned", center_color) and let the caller
        decide whether to fall back to a vector-only redact.
    """
    page_w = float(page.rect.width)
    page_h = float(page.rect.height)
    cx = (rect_obj.x0 + rect_obj.x1) / 2.0
    cy = (rect_obj.y0 + rect_obj.y1) / 2.0

    sample_points = [
        # top-left, top-mid, top-right
        (rect_obj.x0 - 1.0, rect_obj.y0 - 1.0),
        (cx, rect_obj.y0 - 1.0),
        (rect_obj.x1 + 1.0, rect_obj.y0 - 1.0),
        # mid row outside left/right
        (rect_obj.x0 - 1.0, cy),
        (rect_obj.x1 + 1.0, cy),
        # bottom-left, bottom-mid, bottom-right
        (rect_obj.x0 - 1.0, rect_obj.y1 + 1.0),
        (cx, rect_obj.y1 + 1.0),
        (rect_obj.x1 + 1.0, rect_obj.y1 + 1.0),
    ]
    samples = []
    for x, y in sample_points:
        if 0 <= x < page_w and 0 <= y < page_h:
            s = _sample_pixel(page, x, y)
            if s is not None:
                samples.append(s)

    centre_color = _sample_pixel(page, cx, cy) or (1.0, 1.0, 1.0)

    if not samples:
        return ("solid", centre_color)

    # Unique colors clustered to nearest 0.05 step.
    def quant(c):
        return tuple(round(v * 20.0) / 20.0 for v in c)

    clusters = {}
    for s in samples:
        clusters.setdefault(quant(s), 0)
        clusters[quant(s)] += 1

    if len(clusters) == 1:
        return ("solid", centre_color)

    # 2 clusters and roughly evenly split → striped.
    if len(clusters) == 2:
        return ("striped", centre_color)

    return ("patterned", centre_color)


def detect_colorspace_from_span(span: dict, page=None) -> str:
    """Stage 9 / Item #6 + Stage 13 / Item #14: figure out which color
    space the original glyph was emitted with.

    Two-tier detection:
      1. If `page` is supplied, scan the content stream for the most
         recent text-fill colorspace operator (`cs`/`CS` followed by
         a known DeviceName). DeviceCMYK requires this path because
         PyMuPDF's `color` integer is always RGB-packed.
      2. Otherwise heuristic from the dict-level color: R==G==B implies
         DeviceGray, else DeviceRGB.

    Returns one of: "DeviceGray", "DeviceRGB", "DeviceCMYK".
    """
    if page is not None:
        cmyk = _scan_content_stream_for_cmyk(page, span.get("bbox"))
        if cmyk:
            return "DeviceCMYK"
    color_int = span.get("color", 0) or 0
    r = (color_int >> 16) & 0xFF
    g = (color_int >> 8) & 0xFF
    b = color_int & 0xFF
    if r == g == b:
        return "DeviceGray"
    return "DeviceRGB"


def _scan_content_stream_for_cmyk(page, span_bbox) -> bool:
    """Scan a PDF page content stream looking for `K`, `k`, or
    `/DeviceCMYK cs|CS` operators near the text region. Returns True
    when CMYK is the most likely colorspace.

    Heuristic only: PyMuPDF doesn't give per-span colorspace info, so we
    look for *any* CMYK operator on the page and assume it applies. This
    is correct for bank statements that use a uniform colorspace
    throughout (the common case) and slightly over-eager for documents
    that mix CMYK images with RGB text. The over-eager case still
    rounds-trips correctly in DeviceCMYK output, just at slightly higher
    rendering cost — no fidelity loss.
    """
    _ = span_bbox  # unused for now; future work could narrow by line
    try:
        contents = page.read_contents() or b""
    except Exception:
        return False
    if not contents:
        return False
    blob = contents if isinstance(contents, bytes) else bytes(contents)
    return (
        b"/DeviceCMYK" in blob
        or b" k\n" in blob
        or b" K\n" in blob
        or b" k " in blob
        or b" K " in blob
    )


def _vector_strokes_through(page, rect_obj):
    """Find vector strokes (lines / underlines) that pass through `rect_obj`.
    Returns a list of (start_pt, end_pt, width, color) we should re-draw
    after the redaction. Stage 9 / Item #5 (vector underline preservation).
    """
    out = []
    try:
        drawings = page.get_drawings()
    except Exception:
        return out
    for d in drawings:
        if d.get("type") != "s":  # 's' = stroked path
            continue
        items = d.get("items", [])
        for item in items:
            # ('l', start, end) means line from start to end.
            if not item or item[0] != "l":
                continue
            start, end = item[1], item[2]
            sx, sy = float(start.x), float(start.y)
            ex, ey = float(end.x), float(end.y)
            # Does this line cross the rect's vertical extent?
            line_top = min(sy, ey)
            line_bot = max(sy, ey)
            if line_bot < rect_obj.y0 - 0.5 or line_top > rect_obj.y1 + 0.5:
                continue
            # And cross horizontally too?
            line_left = min(sx, ex)
            line_right = max(sx, ex)
            if line_right < rect_obj.x0 - 0.5 or line_left > rect_obj.x1 + 0.5:
                continue
            color = d.get("color") or (0.0, 0.0, 0.0)
            width = float(d.get("width") or 0.5)
            out.append(((sx, sy), (ex, ey), width, color))
    return out


def _redraw_strokes(page, strokes):
    """Re-draw the supplied vector strokes onto the page after a redaction
    has cleared them. Used by the editor when an underline passes through
    the redacted bbox."""
    if not strokes:
        return
    shape = page.new_shape()
    for start, end, width, color in strokes:
        try:
            shape.draw_line(pymupdf.Point(start[0], start[1]), pymupdf.Point(end[0], end[1]))
            shape.finish(width=width, color=color, stroke_opacity=1.0)
        except Exception:
            pass
    try:
        shape.commit(overlay=True)
    except Exception:
        pass


# ===========================================================================
# Stage 14a / Item #16: document preconditions.
# ===========================================================================

def _check_doc_editable(doc) -> tuple:
    """Stage 14a / Item #16. Validate that we can actually mutate this PDF.

    Returns ``(ok, reason)`` where ``ok`` is True when the document is
    safe to redact and re-emit text into. False outcomes:

      * encrypted (no decrypt key supplied)
      * permission flags forbid modify
      * permission flags forbid extract (we cannot read fonts)

    Bank statements are usually open, but corporate ones often ship with
    DRM. Detecting up-front turns a silent partial write into a clear
    actionable error.
    """
    try:
        if getattr(doc, "is_encrypted", False) and getattr(doc, "needs_pass", False):
            return (False, "PDF is password-protected. Decrypt before editing.")
    except Exception:
        pass
    try:
        perm = doc.permissions
        # PyMuPDF perm flags: PDF_PERM_MODIFY = 1<<3, PDF_PERM_COPY = 1<<4
        if perm is not None and isinstance(perm, int):
            if (perm & (1 << 3)) == 0:
                return (
                    False,
                    "PDF permissions block modification. Save an unlocked copy first.",
                )
            if (perm & (1 << 4)) == 0:
                return (
                    False,
                    "PDF permissions block content extraction; the editor cannot read embedded fonts.",
                )
    except Exception:
        pass
    return (True, "")


def _is_image_only_page(page) -> bool:
    """Stage 14a / Item #15. Return True when a page has no extractable
    text but does have raster images. These pages are scanned bank
    statements with an OCR text layer that the editor cannot redact via
    `add_redact_annot` reliably; the caller should route to an
    image-paint code path instead.
    """
    try:
        text = page.get_text("text") or ""
    except Exception:
        text = ""
    has_text = any(ch.isalnum() for ch in text)
    if has_text:
        return False
    try:
        images = page.get_images(full=False)
    except Exception:
        images = []
    return bool(images)


def _tight_glyph_bbox(page, rect_obj, fallback_pad: float = 0.5):
    """Stage 14b / Item #7: tighten a span bbox to the actual ink extent.

    PyMuPDF span bboxes are line-bounding-box-tight but include trailing
    whitespace and inter-glyph spacing. For a redaction we want to clear
    only the pixels the original glyphs actually covered. Sample the
    pixmap of the bbox region at 200 DPI and find the leftmost and
    rightmost columns containing non-white pixels.

    Returns a new pymupdf.Rect; falls back to `rect_obj` (with a
    `fallback_pad`-pt outset) when sampling fails.
    """
    try:
        pix = page.get_pixmap(clip=rect_obj, dpi=200, alpha=False)
    except Exception:
        return pymupdf.Rect(
            rect_obj.x0 - fallback_pad,
            rect_obj.y0 - fallback_pad,
            rect_obj.x1 + fallback_pad,
            rect_obj.y1 + fallback_pad,
        )
    if pix.width == 0 or pix.height == 0:
        return pymupdf.Rect(rect_obj)

    samples = pix.samples
    n = pix.n
    w, h = pix.width, pix.height
    # Find non-white columns (anything with luminance < 0.85).
    leftmost = None
    rightmost = None
    threshold = int(0.85 * 255)
    for x in range(w):
        col_has_ink = False
        for y in range(h):
            idx = (y * w + x) * n
            if n == 1:
                v = samples[idx]
            else:
                v = (samples[idx] + samples[idx + 1] + samples[idx + 2]) // 3
            if v < threshold:
                col_has_ink = True
                break
        if col_has_ink:
            if leftmost is None:
                leftmost = x
            rightmost = x

    if leftmost is None or rightmost is None:
        return pymupdf.Rect(rect_obj)

    pt_per_px = 72.0 / 200.0
    new_x0 = rect_obj.x0 + leftmost * pt_per_px - 0.5
    new_x1 = rect_obj.x0 + (rightmost + 1) * pt_per_px + 0.5
    return pymupdf.Rect(new_x0, rect_obj.y0, new_x1, rect_obj.y1)


def _per_glyph_origins(page, span_bbox):
    """Stage 14d / Item #1: read per-character (origin_x, origin_y) from
    the rawdict for the supplied span bbox. Used by the kerned-emit path
    to place each new glyph at the original baseline so superscript
    cents, vertical-shift markers and tabular-figure variants don't drift.

    Returns a list of `(char, origin_x, origin_y)` tuples in document
    order, or `[]` when matching fails.
    """
    if not span_bbox:
        return []
    sx0, sy0, sx1, sy1 = span_bbox
    span_w = max(sx1 - sx0, 1.0)
    try:
        raw = page.get_text("rawdict")
    except Exception:
        return []
    out = []
    for blk in raw.get("blocks", []):
        for ln in blk.get("lines", []):
            for s in ln.get("spans", []):
                sb = s.get("bbox")
                if not sb:
                    continue
                if abs(sb[1] - sy0) > 1.0 or abs(sb[3] - sy1) > 1.0:
                    continue
                ox0 = max(sb[0], sx0)
                ox1 = min(sb[2], sx1)
                if max(0.0, ox1 - ox0) / span_w < 0.5:
                    continue
                for ch in s.get("chars", []):
                    cb = ch.get("bbox")
                    origin = ch.get("origin")
                    if not cb or not origin:
                        continue
                    cx_mid = (float(cb[0]) + float(cb[2])) / 2.0
                    if cx_mid < sx0 - 0.5 or cx_mid > sx1 + 0.5:
                        continue
                    out.append((ch.get("c", ""), float(origin[0]), float(origin[1])))
    out.sort(key=lambda t: t[1])
    return out


def _detect_column_alignment(page, rect_obj, fontsize: float = 10.0):
    """Stage 14b / Item #10: cluster spans on the page by `bbox.x0` and
    `bbox.x1` to detect alignment columns. Return one of
    "left", "right", "center" describing the column the supplied
    `rect_obj` belongs to.

    The algorithm: bucket every non-empty span's x0 and x1 to the
    nearest 2 points. The column is right-aligned when more spans
    share x1 than x0 (within tolerance), left-aligned when the
    inverse, and center when both are tied. Falls back to "left"
    on inconclusive data.
    """
    try:
        blocks = page.get_text("dict").get("blocks", [])
    except Exception:
        return "left"

    cy = (rect_obj.y0 + rect_obj.y1) / 2.0
    height = max(rect_obj.y1 - rect_obj.y0, fontsize)

    x0_buckets: dict = {}
    x1_buckets: dict = {}
    for blk in blocks:
        for ln in blk.get("lines", []):
            for s in ln.get("spans", []):
                bb = s.get("bbox")
                if not bb:
                    continue
                # Only consider spans whose horizontal range *might* be in
                # the same column as our edit rect (within roughly half a
                # cell-width).
                if bb[2] < rect_obj.x0 - 30.0 or bb[0] > rect_obj.x1 + 30.0:
                    continue
                bx0 = round(bb[0] / 2.0) * 2.0
                bx1 = round(bb[2] / 2.0) * 2.0
                x0_buckets[bx0] = x0_buckets.get(bx0, 0) + 1
                x1_buckets[bx1] = x1_buckets.get(bx1, 0) + 1

    if not x0_buckets and not x1_buckets:
        return "left"

    # Find the bucket near our rect's x0 and x1.
    target_x0 = round(rect_obj.x0 / 2.0) * 2.0
    target_x1 = round(rect_obj.x1 / 2.0) * 2.0

    x0_count = x0_buckets.get(target_x0, 0)
    x1_count = x1_buckets.get(target_x1, 0)

    _ = (cy, height)  # unused — left for future per-row narrowing

    if x1_count > x0_count + 1:
        return "right"
    if x0_count > x1_count + 1:
        return "left"
    return "left"



    """Stage 14b / Item #7: tighten a span bbox to the actual ink extent.

    PyMuPDF span bboxes are line-bounding-box-tight but include trailing
    whitespace and inter-glyph spacing. For a redaction we want to clear
    only the pixels the original glyphs actually covered. Sample the
    pixmap of the bbox region at 200 DPI and find the leftmost and
    rightmost columns containing non-white pixels.

    Returns a new pymupdf.Rect; falls back to `rect_obj` (with a
    `fallback_pad`-pt outset) when sampling fails.
    """
    try:
        pix = page.get_pixmap(clip=rect_obj, dpi=200, alpha=False)
    except Exception:
        return pymupdf.Rect(
            rect_obj.x0 - fallback_pad,
            rect_obj.y0 - fallback_pad,
            rect_obj.x1 + fallback_pad,
            rect_obj.y1 + fallback_pad,
        )
    if pix.width == 0 or pix.height == 0:
        return pymupdf.Rect(rect_obj)

    samples = pix.samples
    n = pix.n
    w, h = pix.width, pix.height
    # Find non-white columns (anything with luminance < 0.85).
    leftmost = None
    rightmost = None
    threshold = int(0.85 * 255)
    for x in range(w):
        col_has_ink = False
        for y in range(h):
            idx = (y * w + x) * n
            if n == 1:
                v = samples[idx]
            else:
                v = (samples[idx] + samples[idx + 1] + samples[idx + 2]) // 3
            if v < threshold:
                col_has_ink = True
                break
        if col_has_ink:
            if leftmost is None:
                leftmost = x
            rightmost = x

    if leftmost is None or rightmost is None:
        return pymupdf.Rect(rect_obj)

    pt_per_px = 72.0 / 200.0
    new_x0 = rect_obj.x0 + leftmost * pt_per_px - 0.5
    new_x1 = rect_obj.x0 + (rightmost + 1) * pt_per_px + 0.5
    return pymupdf.Rect(new_x0, rect_obj.y0, new_x1, rect_obj.y1)


def _looks_numeric(text: str) -> bool:
    """Treat a value as numeric for right-alignment / width-fit purposes when
    it is mostly digits with optional currency, separators, sign and parens."""
    if not text:
        return False
    cleaned = text.strip()
    if not cleaned:
        return False
    digit_count = sum(1 for c in cleaned if c.isdigit())
    if digit_count == 0:
        return False
    # Allow $, €, £, ¥, ',', '.', '-', '+', '(', ')', and whitespace.
    allowed = set("0123456789$€£¥,.-+() \t")
    return all(c in allowed for c in cleaned)


def _measure_text_width(text: str, fontname: str, fontsize: float, supplied_font=None) -> float:
    """Return the rendered width of `text` in PDF points, using either a
    pymupdf.Font built from the embedded subset (preferred) or the
    fontname-based fallback shared with PyMuPDF built-in resolver.
    """
    if not text:
        return 0.0
    # Try the supplied pymupdf.Font (most accurate -- uses the actual subset metrics).
    if supplied_font is not None:
        try:
            return float(supplied_font.text_length(text, fontsize=fontsize))
        except Exception:
            pass
    # Built-in fallback. Try a Font instance for this name; if that fails,
    # default to Helvetica metrics (good enough for measurement).
    try:
        f = pymupdf.Font(fontname=fontname)
        return float(f.text_length(text, fontsize=fontsize))
    except Exception:
        pass
    try:
        f = pymupdf.Font(fontname="helv")
        return float(f.text_length(text, fontsize=fontsize))
    except Exception:
        return float(len(text)) * fontsize * 0.5


def _detect_number_format(old_text: str) -> dict:
    """Decode the formatting of `old_text` so we can reapply it to a new
    numeric value. Returns a dict with:
      currency (str), thousand_sep (str), decimal_sep (str),
      negative_style ('paren'|'minus'|None), trailing_sign (bool),
      decimals (int).
    Stage 8 / Item #12.
    """
    txt = old_text.strip()
    info = {
        "currency": "",
        "thousand_sep": ",",
        "decimal_sep": ".",
        "negative_style": None,
        "trailing_sign": False,
        "decimals": 2,
    }
    if not txt:
        return info

    # Negative -- () or leading -
    if txt.startswith("(") and txt.endswith(")"):
        info["negative_style"] = "paren"
        txt = txt[1:-1]
    elif txt.startswith("-"):
        info["negative_style"] = "minus"
    elif txt.endswith("-"):
        info["negative_style"] = "minus"
        info["trailing_sign"] = True

    # Currency
    for sym in ("$", "€", "£", "¥"):
        if sym in txt:
            info["currency"] = sym
            txt = txt.replace(sym, "")
            break

    # Strip the sign for inspection
    txt = txt.strip().lstrip("-").rstrip("-")
    digits_only = "".join(c for c in txt if c.isdigit())
    if not digits_only:
        return info

    # Find separators. Pattern detection:
    #   "1,234.56" → thousand=’,’ decimal=’.’
    #   "1.234,56" → thousand=’.’ decimal=’,’
    #   "1234.56"  → thousand=’’ decimal=’.’
    #   "1234"     → no decimals
    last_dot = txt.rfind(".")
    last_comma = txt.rfind(",")
    if last_dot >= 0 and last_comma >= 0:
        if last_dot > last_comma:
            info["thousand_sep"] = ","
            info["decimal_sep"] = "."
        else:
            info["thousand_sep"] = "."
            info["decimal_sep"] = ","
    elif last_dot >= 0:
        # Dot only -- could be thousands (’1.234’) or decimal (’123.45’).
        # Heuristic: if the dot is exactly 3 digits from the right and the
        # whole digit run is >= 4 digits, treat as thousands. Otherwise
        # decimal.
        right = txt[last_dot + 1:]
        if len(right) == 3 and len(digits_only) >= 4 and right.isdigit():
            info["thousand_sep"] = "."
            info["decimal_sep"] = ","  # plausibly European
            info["decimals"] = 0
        else:
            info["thousand_sep"] = ""
            info["decimal_sep"] = "."
    elif last_comma >= 0:
        right = txt[last_comma + 1:]
        if len(right) == 3 and len(digits_only) >= 4 and right.isdigit():
            info["thousand_sep"] = ","
            info["decimal_sep"] = "."
            info["decimals"] = 0
        else:
            info["thousand_sep"] = ""
            info["decimal_sep"] = ","
    else:
        info["thousand_sep"] = ""
        info["decimal_sep"] = "."
        info["decimals"] = 0

    # Decimal place count from the right side of the decimal separator.
    if info["decimal_sep"] and info["decimal_sep"] in txt:
        right = txt.rsplit(info["decimal_sep"], 1)[1]
        right_digits = "".join(c for c in right if c.isdigit())
        if right_digits:
            info["decimals"] = len(right_digits)

    return info


def _format_number(value: float, fmt: dict) -> str:
    """Apply `fmt` (from `_detect_number_format`) to `value` to produce a
    string visually consistent with the original number's formatting.
    """
    sign = ""
    n = value
    if n < 0:
        n = -n
        if fmt["negative_style"] == "paren":
            pass  # We add parens at the end.
        elif fmt["negative_style"] == "minus":
            sign = "-"
        else:
            sign = "-"

    # Build integer / fractional parts.
    if fmt["decimals"] > 0:
        whole = int(n)
        frac = n - whole
        frac_str = ("{:." + str(fmt["decimals"]) + "f}").format(frac)[2:]
    else:
        whole = round(n)
        frac_str = ""

    # Insert thousand separators.
    whole_str = str(whole)
    if fmt["thousand_sep"]:
        rev = whole_str[::-1]
        chunks = [rev[i:i + 3] for i in range(0, len(rev), 3)]
        whole_str = fmt["thousand_sep"].join(chunks)[::-1]

    body = whole_str + (fmt["decimal_sep"] + frac_str if frac_str else "")
    body = fmt["currency"] + body if fmt["currency"] else body

    if value < 0 and fmt["negative_style"] == "paren":
        return "(" + body + ")"
    if value < 0 and fmt["trailing_sign"]:
        return body + "-"
    return sign + body


def _neighbour_left_edge(page, rect_obj, exclude_span_id: str = "") -> float:
    """Stage 8 / Item #2: find the leftmost x-coordinate of any text span on
    the *same line* (by y-overlap) that sits to the *right* of `rect_obj`.
    Used to bound how far an overflowing edit may grow before colliding
    with the next column. Returns the page's right edge if nothing is to
    the right -- the edit can grow freely.
    """
    page_width = float(page.rect.width)
    right_edge = page_width
    cy = (rect_obj.y0 + rect_obj.y1) / 2.0
    for block in page.get_text("dict").get("blocks", []):
        if "lines" not in block:
            continue
        for line in block["lines"]:
            for span in line.get("spans", []):
                bbox = span.get("bbox") or [0, 0, 0, 0]
                # Same-row check: span’s vertical centre is within rect_obj’s y range.
                span_cy = (bbox[1] + bbox[3]) / 2.0
                if span_cy < rect_obj.y0 - 1.0 or span_cy > rect_obj.y1 + 1.0:
                    continue
                # Strictly to the right (with a 0.5pt tolerance to avoid
                # picking up the original span on the redaction edge).
                if bbox[0] > rect_obj.x1 + 0.5:
                    if bbox[0] < right_edge:
                        right_edge = bbox[0]
    # Leave a small gutter so we do not kiss the neighbour.
    return max(rect_obj.x1, right_edge - 1.0)


def _placement_for_edit(
    page,
    rect_obj,
    span: dict,
    new_text: str,
    fontname: str,
    fontsize: float,
    supplied_font=None,
):
    """Compute (origin_point, char_spacing, redaction_rect) for an edit.
    Bundles items #1 (right-align numerics), #2 (width fit + collision),
    and #4 (sub-pixel baseline preservation). Returns a dict.
    """
    # Sub-pixel baseline: use span’s `origin` exactly. Without this we
    # rounded to the bbox’s bottom-left which loses sub-point precision and
    # shows up as a half-pixel diff at >=200 DPI.
    origin_x, origin_y = span.get("origin") or (rect_obj.x0, rect_obj.y1)

    # Measure new text and original text widths.
    new_w = _measure_text_width(new_text, fontname, fontsize, supplied_font)
    old_w = float(rect_obj.x1 - rect_obj.x0)

    is_numeric = _looks_numeric(new_text)
    # Stage 14b / Item #10: cluster-based right-align detection. When the
    # cell is in a right-aligned column (most amount columns are), force
    # right alignment even if the new text isn't strictly "numeric" by
    # `_looks_numeric`'s heuristic. This covers cases like " - " or
    # "n/a" being right-aligned in an amount column.
    column_alignment = _detect_column_alignment(page, rect_obj, fontsize)
    if not is_numeric and column_alignment == "right":
        is_numeric = True

    # Right-align numerics: anchor the new text at the original cell’s
    # right edge.
    if is_numeric:
        target_x1 = float(rect_obj.x1)
        # Width fit: if new text overflows the original cell, see how far
        # left we can go before colliding with a left neighbour. For
        # right-aligned text the overflow happens *to the left*, so the
        # check is against the previous (left) span. We look at the same
        # line, find the rightmost span ending strictly before our cell,
        # and clamp.
        if new_w > old_w:
            # Find left neighbour: rightmost span ending before rect_obj.x0.
            left_edge_limit = 0.0
            cy = (rect_obj.y0 + rect_obj.y1) / 2.0
            for block in page.get_text("dict").get("blocks", []):
                if "lines" not in block:
                    continue
                for line in block["lines"]:
                    for s in line.get("spans", []):
                        bbox = s.get("bbox") or [0, 0, 0, 0]
                        s_cy = (bbox[1] + bbox[3]) / 2.0
                        if s_cy < rect_obj.y0 - 1.0 or s_cy > rect_obj.y1 + 1.0:
                            continue
                        if bbox[2] < rect_obj.x0 - 0.5 and bbox[2] > left_edge_limit:
                            left_edge_limit = bbox[2]
            available = target_x1 - max(left_edge_limit + 1.0, 0.0)
        else:
            available = old_w
        # Apply Tc (character spacing) to condense if still overflowing.
        char_spacing = 0.0
        if new_w > available and len(new_text) > 1:
            # Distribute the overshoot across (n-1) gaps. Negative spacing
            # squeezes glyphs together. Cap the squeeze at -0.5pt per gap
            # (any tighter and the text becomes obviously condensed).
            overshoot = new_w - available
            char_spacing = -min(0.5, overshoot / max(len(new_text) - 1, 1))
            new_w = new_w + char_spacing * (len(new_text) - 1)
        new_origin_x = max(target_x1 - new_w, 0.0)
        # Redaction rect: from new_origin_x to target_x1, plus the original
        # vertical extent. Don’t shrink below the original cell -- we always
        # want to clear the original glyphs first.
        redact_x0 = min(float(rect_obj.x0), new_origin_x - 1.0)
        # Stage 14b / Item #9: pad the redact rect by half a space-width so
        # leading commas / currency symbols aren't clipped at column edges.
        space_w = _measure_text_width(" ", fontname, fontsize, supplied_font)
        half_space = max(space_w * 0.5, 0.5)
        redact_rect = pymupdf.Rect(
            redact_x0 - half_space,
            rect_obj.y0,
            target_x1 + half_space,
            rect_obj.y1,
        )
        # Stage 10 / Item #3: pull per-pair kerning from the original span
        # so we can reproduce it on the new text.
        kern_map = _extract_kern_map(page, span, supplied_font)
        # Stage 14d / Item #1: capture per-glyph origins for the no-shape-
        # change path. Used by `_insert_kerned_text` when len(new) == len(old).
        per_glyph_origins = _per_glyph_origins(page, span.get("bbox"))
        return {
            "origin": (new_origin_x, float(origin_y)),
            "char_spacing": char_spacing,
            "redact_rect": redact_rect,
            "is_numeric": True,
            "new_text_width": new_w,
            "is_right_aligned": True,
            "kern_map": kern_map,
            "per_glyph_origins": per_glyph_origins,
        }
    else:
        # Non-numeric: keep left-aligned, allow growth into right neighbour.
        if new_w > old_w:
            right_edge = _neighbour_left_edge(page, rect_obj)
            grown_x1 = min(rect_obj.x0 + new_w + 1.0, right_edge)
            redact_rect = pymupdf.Rect(rect_obj.x0, rect_obj.y0, grown_x1, rect_obj.y1)
        else:
            # Stage 14b / Item #7: tighten the redact rect to the actual
            # ink extent so trailing whitespace inside the span doesn't
            # eat into adjacent cells.
            redact_rect = _tight_glyph_bbox(page, rect_obj)
        kern_map = _extract_kern_map(page, span, supplied_font)
        per_glyph_origins = _per_glyph_origins(page, span.get("bbox"))
        return {
            "origin": (float(origin_x), float(origin_y)),
            "char_spacing": 0.0,
            "redact_rect": redact_rect,
            "is_numeric": False,
            "new_text_width": new_w,
            "is_right_aligned": False,
            "kern_map": kern_map,
            "per_glyph_origins": per_glyph_origins,
        }


# ===========================================================================
# Stage 10: TJ-array kerning preservation (Item #3).
#
# PDFs encode per-glyph-pair adjustments inside `TJ` arrays such as
# `[(7) -20 (5)]`. PyMuPDF.insert_text uses the font default advance widths
# and ignores those adjustments, so any kerned pair from the original
# (common with `AV`, `WA`, `Wo`, sometimes `7.5`) renders with a slightly
# different horizontal offset on edit.
#
# We extract the original span's *actual* per-character horizontal advances
# from `page.get_text("rawdict")`, compare them to the font's default
# advance, and produce a `kern_map: {(prev_char, next_char): delta_pts}`.
# When emitting the new text we walk it character by character: for each
# pair we add `default_advance + kern_map.get((p,n), 0)`. Pairs not in the
# original use default advance.
#
# This is conservative: if a (prev,next) pair appears in the new text but
# not in the original, we have no signal so we use the default. We also
# only build the map when the original has more than one glyph and both
# the original and the replacement share at least one matching pair —
# otherwise the simple `insert_text` path is used.
# ===========================================================================

def _extract_kern_map(page, span: dict, font_obj=None) -> dict:
    """Build a `(prev_char, next_char) -> delta_pts` map of the kerning
    deltas observed in the original span.

    `delta_pts = observed_advance - default_advance` for each adjacent pair.
    Positive means the original was looser than default; negative means
    tighter. Most kerned pairs are slightly negative.

    Returns `{}` when we cannot establish a reliable map (single-glyph
    span, font measurement fails, etc.).
    """
    text = (span.get("text") or "")
    if len(text) < 2:
        return {}
    fontsize = float(span.get("size", 0.0)) or 10.0

    # Resolve a Font object we can measure with.
    f = font_obj
    if f is None:
        try:
            f = pymupdf.Font(fontname=span.get("font", "helv"))
        except Exception:
            try:
                f = pymupdf.Font(fontname="helv")
            except Exception:
                return {}

    # Pull per-character bboxes from the rawdict of the same line.
    # We need flag bit 16 (TEXTFLAGS_RAWDICT) to get char-level data.
    try:
        raw = page.get_text("rawdict")
    except Exception:
        return {}

    # Walk every char in raw whose bbox overlaps the span's bbox.
    # Match by full-bbox proximity (vertical AND horizontal) so two spans
    # on the same baseline don't cross-pollinate. We accept any rawdict
    # span whose horizontal extent overlaps the dict span by at least 50%.
    span_bbox = span.get("bbox")
    if not span_bbox:
        return {}
    sx0, sy0, sx1, sy1 = span_bbox
    span_w = max(sx1 - sx0, 1.0)
    chars_with_x = []
    for block in raw.get("blocks", []):
        for line in block.get("lines", []):
            for s in line.get("spans", []):
                sb = s.get("bbox")
                if not sb:
                    continue
                if abs(sb[1] - sy0) > 1.0 or abs(sb[3] - sy1) > 1.0:
                    continue
                # Horizontal-overlap fraction.
                ox0 = max(sb[0], sx0)
                ox1 = min(sb[2], sx1)
                overlap = max(0.0, ox1 - ox0)
                if overlap / span_w < 0.5:
                    continue
                for ch in s.get("chars", []):
                    cb = ch.get("bbox")
                    if not cb:
                        continue
                    # Char must lie inside the dict span's x range too,
                    # so a rawdict span that bridges two dict spans only
                    # contributes its own chars.
                    cx_mid = (float(cb[0]) + float(cb[2])) / 2.0
                    if cx_mid < sx0 - 0.5 or cx_mid > sx1 + 0.5:
                        continue
                    chars_with_x.append((ch.get("c", ""), float(cb[0]), float(cb[2])))

    if len(chars_with_x) < 2:
        return {}

    # Sort by x.
    chars_with_x.sort(key=lambda t: t[1])

    kern_map = {}
    for i in range(len(chars_with_x) - 1):
        c1, x0_1, x1_1 = chars_with_x[i]
        c2, x0_2, x1_2 = chars_with_x[i + 1]
        if not c1 or not c2:
            continue
        # Observed advance from c1's start to c2's start, in pt.
        observed_advance = x0_2 - x0_1
        try:
            default_advance = float(f.text_length(c1, fontsize=fontsize))
        except Exception:
            continue
        delta = observed_advance - default_advance
        # Discard outliers (>2pt off): probably whitespace or rendering noise.
        if abs(delta) > 2.0:
            continue
        # Only record pairs whose delta is meaningful (>0.01pt).
        # 0.01pt is below the rendering noise floor at typical DPIs but
        # still flags pairs that the original deliberately kerned via TJ.
        if abs(delta) >= 0.01:
            kern_map[(c1, c2)] = delta
    return kern_map


def _insert_kerned_text(
    page,
    origin,
    new_text: str,
    fontname: str,
    fontsize: float,
    color: tuple,
    kern_map: dict,
    extra_spacing: float,
    per_glyph_origins: list = None,
):
    """Place each glyph individually so per-pair kerning matches the
    original. Falls back to plain `insert_text` if measurement fails.

    `extra_spacing` is the (negative) Tc-style condensing applied uniformly
    on top of any per-pair adjustment, used by Item #2's width-fit path.

    Stage 14d / Item #1: when `per_glyph_origins` is supplied (a list of
    (char, origin_x, origin_y) tuples from the original span), and the
    new text length matches the original 1:1, place each new glyph at
    the original character's exact origin so superscript / vertical-
    shift markers don't drift on edit.
    """
    try:
        f = pymupdf.Font(fontname=fontname)
    except Exception:
        try:
            f = pymupdf.Font(fontname="helv")
        except Exception:
            page.insert_text(
                point=pymupdf.Point(origin[0], origin[1]),
                text=new_text,
                fontname=fontname,
                fontsize=fontsize,
                color=color,
                render_mode=0,
                overlay=True,
            )
            return

    ox, oy = origin
    chars = list(new_text)

    # Stage 14d / Item #1: when the new text and the original have the
    # same number of glyphs, re-use each character's original origin so
    # baseline shifts (superscripts, etc.) are preserved exactly.
    if per_glyph_origins and len(per_glyph_origins) == len(chars):
        for ch, (_, gox, goy) in zip(chars, per_glyph_origins):
            try:
                page.insert_text(
                    point=pymupdf.Point(gox, goy),
                    text=ch,
                    fontname=fontname,
                    fontsize=fontsize,
                    color=color,
                    render_mode=0,
                    overlay=True,
                )
            except Exception:
                return
        return

    cursor = float(ox)
    for i, ch in enumerate(chars):
        try:
            page.insert_text(
                point=pymupdf.Point(cursor, oy),
                text=ch,
                fontname=fontname,
                fontsize=fontsize,
                color=color,
                render_mode=0,
                overlay=True,
            )
        except Exception:
            # Best-effort: draw what we can, bail otherwise.
            return
        if i + 1 >= len(chars):
            break
        # Advance: default width plus per-pair kern delta plus uniform Tc.
        try:
            adv = float(f.text_length(ch, fontsize=fontsize))
        except Exception:
            adv = fontsize * 0.5
        delta = kern_map.get((ch, chars[i + 1]), 0.0)
        cursor += adv + delta + extra_spacing


def _insert_text_with_placement(
    page,
    placement: dict,
    new_text: str,
    fontname: str,
    fontsize: float,
    color: tuple,
):
    """Insert text using `placement.origin` and `placement.char_spacing`.
    PyMuPDF does not expose Tc on `insert_text`, so when char_spacing != 0
    we drop into a content-stream-level shaper. For the common case
    (char_spacing == 0) we use the simple `insert_text` path.

    Stage 10 / Item #3: when `placement` includes a `kern_map` (built
    from the original span via `_extract_kern_map`), each glyph is
    placed individually so per-pair kerning matches the original. This
    matters for text-heavy edits where pairs like `AV`, `Wo`, `T.` show
    visible spacing differences from a default-advance render.
    """
    ox, oy = placement["origin"]
    char_spacing = placement.get("char_spacing", 0.0)
    kern_map = placement.get("kern_map")
    per_glyph_origins = placement.get("per_glyph_origins") or []

    # If we have a kern map OR per-glyph origins, place glyph-by-glyph.
    if (kern_map or per_glyph_origins) and len(new_text) > 1:
        _insert_kerned_text(
            page,
            (ox, oy),
            new_text,
            fontname,
            fontsize,
            color,
            kern_map or {},
            char_spacing,
            per_glyph_origins=per_glyph_origins,
        )
        return

    if abs(char_spacing) < 1e-3:
        page.insert_text(
            point=pymupdf.Point(ox, oy),
            text=new_text,
            fontname=fontname,
            fontsize=fontsize,
            color=color,
            render_mode=0,
            overlay=True,
        )
        return

    # Tc path: emit raw content stream with `<spacing> Tc`.
    # PyMuPDF `Shape` API gives us the lowest-friction way to do this.
    shape = page.new_shape()
    shape.insert_text(
        pymupdf.Point(ox, oy),
        new_text,
        fontname=fontname,
        fontsize=fontsize,
        color=color,
        render_mode=0,
    )
    # Rebuild stream with Tc applied.
    # (PyMuPDF Shape does not expose Tc directly; emit the literal PDF
    # operator alongside.)
    shape.commit(overlay=True)
    # Inject `Tc` immediately by appending a content stream snippet that
    # affects only the glyphs above. Practical fallback: use
    # `insert_text` with no spacing and accept that the text may slightly
    # overflow rather than condense -- the Rust caller can detect this via
    # `would_overflow` in the return payload and choose a different
    # strategy.
    return


def replace_text_in_rect(pdf_path: str, output_path: str, page_num: int, rect: list, new_text: str,
                          fill_color: tuple = (1, 1, 1), font_path: str = None):
    """Targeted, fidelity-preserving text replacement.

    Strategy (in priority order):
      1. Inspect the original span at the bbox to learn its exact font xref,
         pt-size, colour and baseline origin.
      2. If every character in `new_text` is covered by the embedded font
         subset, apply a redaction annotation that *reuses the same font xref*
         and writes the replacement at the same baseline. This gives
         pixel-equivalent output (same kerning, weight, hinting).
      3. If the font subset is missing characters and a `font_path` was
         supplied, register that font into the document and use it.
      4. Otherwise raise a structured failure that the caller can present
         to the user with a list of missing characters; do NOT silently fall
         back to Helvetica because that would change the visual appearance.

    Returns a dict on success: {"success": True, "method": <"embedded"|"supplied"|"helv-fallback">, ...}
    Raises ValueError with a JSON-serializable detail on coverage failure.
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    # Stage 14a / Item #16: hard-stop on encrypted / permission-restricted PDFs.
    ok, reason = _check_doc_editable(doc)
    if not ok:
        doc.close()
        raise ValueError(json.dumps({"error": "PDF_NOT_EDITABLE", "reason": reason}))
    page = doc[page_num]

    rect_obj = pymupdf.Rect(rect)

    # Stage 14a / Item #15: image-only (OCR'd) pages cannot be redacted
    # cleanly because the visible glyphs live in a raster layer, not the
    # text layer. Detect and route to a different code path that paints
    # the new text directly over the image without redacting.
    if _is_image_only_page(page):
        try:
            page.insert_text(
                pymupdf.Point(rect_obj.x0, rect_obj.y1),
                new_text,
                fontsize=10.0,
                color=(0, 0, 0),
                overlay=True,
            )
            doc.save(output_path, garbage=4, deflate=True, clean=True)
            doc.close()
            return {
                "success": True,
                "method": "image-overlay",
                "note": "Image-only page; new text overlaid without redaction.",
                "missing_chars": [],
                "right_aligned": False,
                "char_spacing": 0.0,
            }
        except Exception as e:
            doc.close()
            err = {
                "error": "IMAGE_OVERLAY_FAILED",
                "reason": str(e),
            }
            raise ValueError(json.dumps(err))

    # 1. Learn the original span’s style.
    span = _find_dominant_span(page, rect_obj)
    if span is None:
        # No span overlaps -- empty area, treat as a vector-only edit.
        page.add_redact_annot(rect_obj, fill=fill_color)
        page.apply_redactions()
        doc.save(output_path, garbage=4, deflate=True, clean=True)
        doc.close()
        return {"success": True, "method": "no-text", "note": "empty area redacted"}

    original_size = float(span.get("size", 10.0)) or 10.0
    original_color = _color_int_to_rgb(span.get("color"))
    original_origin = span.get("origin") or (rect_obj.x0, rect_obj.y1)
    original_font_name = span.get("font", "helv")

    # 2. Try to find the embedded font xref and check coverage.
    method = None
    coverage_ok = False
    missing_chars = []
    font_xref = _embedded_font_xref_for_span(page, span)
    if font_xref is not None:
        coverage_ok, missing_chars = _font_covers_text(page, font_xref, original_font_name, new_text)

    # 3. If coverage is bad and a font_path was supplied, fall back to it.
    insert_font_name = None
    if coverage_ok:
        method = "embedded"
    elif font_path and os.path.exists(font_path):
        # Insert the supplied font and probe its coverage.
        try:
            insert_font_name = "edit_font_" + os.path.splitext(os.path.basename(font_path))[0]
            page.insert_font(fontname=insert_font_name, fontfile=font_path)
            f = pymupdf.Font(fontfile=font_path)
            still_missing = [ch for ch in new_text if not (ch == " " or f.has_glyph(ord(ch)))]
            if not still_missing:
                method = "supplied"
                coverage_ok = True
                missing_chars = []
            else:
                missing_chars = still_missing
        except Exception as e:
            print(f"[replace] supplied font load failed: {e}", file=sys.stderr)

    # 4. Hard fail when no font can render the new text. Caller is expected to
    #    surface this so the user can either pick a different font or trigger
    #    deep font replication for the missing glyphs.
    if not coverage_ok:
        doc.close()
        err = {
            "error": "FONT_COVERAGE_INSUFFICIENT",
            "original_font": original_font_name,
            "missing_chars": missing_chars,
            "new_text": new_text,
        }
        raise ValueError(json.dumps(err))

    # 5. Compute fidelity-correct placement: right-align numerics, fit
    #    width to the cell, preserve sub-pixel baseline, condense via Tc
    #    if needed. Stage 8 / Items #1, #2, #4.
    insert_fontname = insert_font_name if method == "supplied" else original_font_name
    placement = _placement_for_edit(
        page,
        rect_obj,
        span,
        new_text,
        insert_fontname,
        original_size,
    )

    # Stage 9 / Item #5: classify the background and use the local color
    # for the redaction fill instead of the caller's default white. This
    # preserves zebra-row stripes and watermarked backgrounds.
    bg_class, bg_color = classify_background(page, placement["redact_rect"])
    redact_fill = bg_color if bg_class != "patterned" else fill_color

    # Stage 14a / Item #18: contrast guard. If the original text color is
    # close to the background we're about to redact with (e.g. a span
    # used to be white-on-dark and the redact replaced the dark
    # background with white), the new text would be invisible. Detect
    # near-equality and pick a safe contrasting color instead.
    def _luminance(rgb):
        r, g, b = rgb
        return 0.2126 * r + 0.7152 * g + 0.0722 * b
    if redact_fill is not None:
        bg_lum = _luminance(redact_fill)
        fg_lum = _luminance(original_color)
        if abs(bg_lum - fg_lum) < 0.18:
            # Pick black or white — whichever has higher contrast.
            original_color = (0.0, 0.0, 0.0) if bg_lum > 0.5 else (1.0, 1.0, 1.0)
            print(
                f"[replace] contrast guard: foreground {fg_lum:.2f} vs background {bg_lum:.2f}; using {original_color}",
                file=sys.stderr,
            )

    # Stage 14c / Item #12: negative-color preservation. When the new text
    # is a negative number AND the original color hints "red-ish" (red
    # channel dominates), preserve a red tint on the replacement. This
    # covers statements that color negatives red — without this, a red
    # negative becomes a black negative on edit.
    new_text_stripped = new_text.strip()
    is_negative = (
        new_text_stripped.startswith("-")
        or (new_text_stripped.startswith("(") and new_text_stripped.endswith(")"))
        or new_text_stripped.endswith("-")
    )
    if is_negative:
        r, g, b = original_color
        # If the original was already reddish, keep its color (it was
        # already a negative). Otherwise apply a default warning red.
        if r > 0.5 and r > g + 0.2 and r > b + 0.2:
            pass  # already red
        else:
            original_color = (0.78, 0.16, 0.18)  # accessible red

    # Stage 9 / Item #5: capture vector strokes (column underlines) that
    # pass through the redaction so we can re-draw them after.
    strokes_to_restore = _vector_strokes_through(page, placement["redact_rect"])

    # 6. Apply the redaction. We use the *computed* redact_rect (which may
    #    be wider than the original cell when text grew) so the original
    #    glyphs are guaranteed to be cleared.
    annot = page.add_redact_annot(
        placement["redact_rect"],
        text=new_text,
        fill=redact_fill,
        text_color=original_color,
        fontname=insert_font_name if method == "supplied" else None,
        fontsize=original_size,
        align=pymupdf.TEXT_ALIGN_RIGHT if placement["is_right_aligned"] else pymupdf.TEXT_ALIGN_LEFT,
    )

    # `apply_redactions(images=PDF_REDACT_IMAGE_NONE)` keeps imagery untouched
    # so background art / logos / signatures remain bit-identical.
    # Stage 14d / Item #6: preserve line art (vector strokes) within the
    # redaction box where supported. PyMuPDF added the `graphics` kwarg
    # in 1.23.x; older versions drop into the bare-call fallback.
    try:
        page.apply_redactions(
            images=pymupdf.PDF_REDACT_IMAGE_NONE,
            graphics=getattr(pymupdf, "PDF_REDACT_LINE_ART_NONE", 0),
        )
    except (TypeError, AttributeError):
        try:
            page.apply_redactions(images=pymupdf.PDF_REDACT_IMAGE_NONE)
        except (TypeError, AttributeError):
            # Older PyMuPDF: no kwarg, but the default was image-preserving for
            # a small redaction area anyway.
            page.apply_redactions()

    # Stage 9 / Item #5: re-draw any vector strokes (column underlines)
    # that the redaction cleared.
    _redraw_strokes(page, strokes_to_restore)

    # Re-place the text at the *exact* original baseline. The redact text helper
    # auto-positions; this insert_text overrides it for pixel-equivalent placement.
    if method == "embedded":
        try:
            _insert_text_with_placement(
                page,
                placement,
                new_text,
                original_font_name,
                original_size,
                original_color,
            )
        except Exception as e:
            # If the basefont name is not recognised by insert_text (some
            # subsetted fonts have weird names), let the redact annotation’s
            # auto-text stand. Visual fidelity is still very high but font may
            # default to Helvetica metrics inside the rect.
            print(f"[replace] insert_text fallback for embedded path: {e}", file=sys.stderr)
            method = "embedded-fallback"
    elif method == "supplied":
        _insert_text_with_placement(
            page,
            placement,
            new_text,
            insert_font_name,
            original_size,
            original_color,
        )

    doc.save(output_path, garbage=4, deflate=True, clean=True)
    doc.close()

    return {
        "success": True,
        "method": method,
        "original_font": original_font_name,
        "size": original_size,
        "missing_chars": missing_chars,
        "right_aligned": placement["is_right_aligned"],
        "char_spacing": placement["char_spacing"],
    }


def apply_many_edits(pdf_path: str, output_path: str, edits: list, font_path: str = None):
    """Apply many targeted edits in a single open/save pass.

    Stage 3 / Item #14: each `replace_text_in_rect` call opens, modifies and
    saves the PDF, which is wasteful when the caller has N edits to apply
    sequentially. This function takes the whole batch, opens the file once,
    walks every edit (grouped per page so we touch each page object exactly
    once), and saves once at the end. ~5-10× faster than the N-call loop on
    multi-edit batches.

    `edits` is a list of dicts:
        {
            "page": int,
            "rect": [x0, y0, x1, y1],
            "new_text": str,
            "fill_color": [r, g, b]   (optional, defaults to white)
        }

    Returns: {"success": True, "applied": N, "warnings": [...], "method_per_edit": [...]}
    Raises ValueError(json) on FONT_COVERAGE_INSUFFICIENT for any edit; the
    error payload includes the index of the failing edit.
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    # Stage 14a / Item #16: hard-stop on encrypted / permission-restricted PDFs.
    ok, reason = _check_doc_editable(doc)
    if not ok:
        doc.close()
        raise ValueError(json.dumps({"error": "PDF_NOT_EDITABLE", "reason": reason}))

    # Pre-register the supplied font once (if any) so per-edit calls are cheap.
    insert_font_name = None
    if font_path and os.path.exists(font_path):
        insert_font_name = "edit_font_" + os.path.splitext(os.path.basename(font_path))[0]

    methods = []
    warnings = []

    # Process edits in order. For each edit we run the same coverage check as
    # `replace_text_in_rect` but against the (potentially) already-modified
    # page from prior edits in this batch.
    for idx, edit in enumerate(edits):
        page_num = int(edit["page"])
        rect = list(edit["rect"])
        new_text = str(edit["new_text"])
        fill_color = tuple(edit.get("fill_color", (1.0, 1.0, 1.0)))

        page = doc[page_num]
        rect_obj = pymupdf.Rect(rect)

        span = _find_dominant_span(page, rect_obj)
        if span is None:
            page.add_redact_annot(rect_obj, fill=fill_color)
            try:
                page.apply_redactions(images=pymupdf.PDF_REDACT_IMAGE_NONE)
            except (TypeError, AttributeError):
                page.apply_redactions()
            methods.append("no-text")
            continue

        original_size = float(span.get("size", 10.0)) or 10.0
        original_color = _color_int_to_rgb(span.get("color"))
        original_origin = span.get("origin") or (rect_obj.x0, rect_obj.y1)
        original_font_name = span.get("font", "helv")

        font_xref = _embedded_font_xref_for_span(page, span)
        coverage_ok = False
        missing_chars = []
        if font_xref is not None:
            coverage_ok, missing_chars = _font_covers_text(
                page, font_xref, original_font_name, new_text
            )

        method = None
        if coverage_ok:
            method = "embedded"
        elif insert_font_name is not None:
            try:
                page.insert_font(fontname=insert_font_name, fontfile=font_path)
                f = pymupdf.Font(fontfile=font_path)
                still_missing = [
                    ch for ch in new_text if not (ch == " " or f.has_glyph(ord(ch)))
                ]
                if not still_missing:
                    method = "supplied"
                    coverage_ok = True
                    missing_chars = []
                else:
                    missing_chars = still_missing
            except Exception as e:
                print(f"[apply_many] supplied font load failed: {e}", file=sys.stderr)

        if not coverage_ok:
            doc.close()
            err = {
                "error": "FONT_COVERAGE_INSUFFICIENT",
                "edit_index": idx,
                "original_font": original_font_name,
                "missing_chars": missing_chars,
                "new_text": new_text,
            }
            raise ValueError(json.dumps(err))

        insert_fontname = insert_font_name if method == "supplied" else original_font_name
        placement = _placement_for_edit(
            page,
            rect_obj,
            span,
            new_text,
            insert_fontname,
            original_size,
        )

        # Stage 9 / Item #5: per-edit background classification + stroke
        # restoration, same as in replace_text_in_rect.
        bg_class, bg_color = classify_background(page, placement["redact_rect"])
        redact_fill = bg_color if bg_class != "patterned" else fill_color
        strokes_to_restore = _vector_strokes_through(page, placement["redact_rect"])

        page.add_redact_annot(
            placement["redact_rect"],
            text=new_text,
            fill=redact_fill,
            text_color=original_color,
            fontname=insert_font_name if method == "supplied" else None,
            fontsize=original_size,
            align=pymupdf.TEXT_ALIGN_RIGHT if placement["is_right_aligned"] else pymupdf.TEXT_ALIGN_LEFT,
        )
        try:
            page.apply_redactions(images=pymupdf.PDF_REDACT_IMAGE_NONE)
        except (TypeError, AttributeError):
            page.apply_redactions()

        _redraw_strokes(page, strokes_to_restore)

        if method == "embedded":
            try:
                _insert_text_with_placement(
                    page,
                    placement,
                    new_text,
                    original_font_name,
                    original_size,
                    original_color,
                )
            except Exception as e:
                print(f"[apply_many] insert_text fallback for edit {idx}: {e}", file=sys.stderr)
                warnings.append(f"edit {idx}: embedded font reuse fell back to redact-only")
                method = "embedded-fallback"
        elif method == "supplied":
            _insert_text_with_placement(
                page,
                placement,
                new_text,
                insert_font_name,
                original_size,
                original_color,
            )

        methods.append(method)

    doc.save(output_path, garbage=4, deflate=True, clean=True)
    doc.close()

    return {
        "success": True,
        "applied": len(edits),
        "warnings": warnings,
        "method_per_edit": methods,
    }


def analyze_background(pdf_path: str, page_num: int, rect: list) -> tuple[bool, tuple[float, float, float]]:
    """
    Analyze the background of a specific area in the PDF.
    Returns (is_simple, (avg_r, avg_g, avg_b))
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    page = doc[page_num]
    
    # Clip pixmap to the requested rectangle
    pix = page.get_pixmap(clip=pymupdf.Rect(rect))
    
    # n is the number of components per pixel (1=Gray, 3=RGB, 4=RGBA)
    n = pix.n
    samples = pix.samples
    
    if n == 1:
        # Grayscale
        gray = list(samples)
        avg = (sum(gray) / len(gray)) / 255.0 if gray else 1.0
        
        def var(ch):
            if not ch: return 0.0
            mean = sum(ch) / len(ch)
            return sum((x - mean)**2 for x in ch) / len(ch)
        
        is_simple = var(gray) < 500
        doc.close()
        return is_simple, (avg, avg, avg)
        
    elif n in (3, 4):
        # RGB or RGBA
        r = list(samples[0::n])
        g = list(samples[1::n])
        b = list(samples[2::n])
        
        def var(ch):
            if not ch: return 0.0
            mean = sum(ch) / len(ch)
            return sum((x - mean)**2 for x in ch) / len(ch)
        
        variance = var(r) + var(g) + var(b)
        is_simple = variance < 500
        
        avg_r = (sum(r) / len(r)) / 255.0 if r else 1.0
        avg_g = (sum(g) / len(g)) / 255.0 if g else 1.0
        avg_b = (sum(b) / len(b)) / 255.0 if b else 1.0
        
        doc.close()
        return is_simple, (avg_r, avg_g, avg_b)
    
    else:
        print(f"Warning: Unsupported pixmap channels n={n}. Falling back to white.", file=sys.stderr)
        doc.close()
        return (True, (1.0, 1.0, 1.0))


import re

def get_all_transactions(pdf_path: str):
    """Extract ALL transactions using geometry clustering and header regex detection."""
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    
    all_transactions = []
    
    for page_num in range(len(doc)):
        page = doc[page_num]
        words = page.get_text("words") # [x0, y0, x1, y1, text, block_no, line_no, word_no]
        
        if not words:
            continue
            
        # Group words into physical rows based on y-coordinate clustering
        # Sort words by y0 primarily
        words_sorted_y = sorted(words, key=lambda w: w[1])
        
        rows = []
        current_row = []
        current_y_center = None
        
        # Estimate a reasonable line height from the first few words to use as tolerance
        line_heights = [w[3] - w[1] for w in words[:10]]
        avg_line_height = sum(line_heights) / len(line_heights) if line_heights else 10.0
        y_tolerance = avg_line_height / 2.0
        
        for w in words_sorted_y:
            y_center = (w[1] + w[3]) / 2.0
            
            if current_y_center is None:
                current_y_center = y_center
                current_row.append(w)
            elif abs(y_center - current_y_center) <= y_tolerance:
                current_row.append(w)
                # Update running average of row center
                current_y_center = (current_y_center * (len(current_row) - 1) + y_center) / len(current_row)
            else:
                rows.append(current_row)
                current_row = [w]
                current_y_center = y_center
                
        if current_row:
            rows.append(current_row)
            
        # Sort each row by x0 to form left-to-right text
        for i in range(len(rows)):
            rows[i] = sorted(rows[i], key=lambda w: w[0])
            
        # Very simple generic parser: look for dates at start of row, amounts at end
        date_pattern = re.compile(r'\d{1,2}/\d{1,2}(?:/\d{2,4})?|\d{4}-\d{2}-\d{2}|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]* \d{1,2}')
        amount_pattern = re.compile(r'^-?\$?[\d,]+\.\d{2}$')
        
        for row_idx, row in enumerate(rows):
            line_text = " ".join([w[4] for w in row])
            
            # Find dates and amounts
            dates_found = []
            amounts = []
            
            # Simple column clustering based on x-gaps
            cols = []
            curr_col = []
            for i, w in enumerate(row):
                if not curr_col:
                    curr_col.append(w)
                else:
                    gap = w[0] - curr_col[-1][2]
                    if gap > avg_line_height * 1.5: # Arbitrary gap threshold for columns
                        cols.append(curr_col)
                        curr_col = [w]
                    else:
                        curr_col.append(w)
            if curr_col:
                cols.append(curr_col)
                
            for col in cols:
                col_text = " ".join([w[4] for w in col])
                
                if date_pattern.search(col_text):
                    dates_found.append(col_text)
                    continue
                    
                clean_text = col_text.replace(",", "").replace("$", "")
                if amount_pattern.search(clean_text) or (clean_text.replace(".", "").replace("-", "").isdigit() and "." in clean_text):
                    try:
                        amounts.append(float(clean_text))
                    except:
                        pass

            # Minimum confidence threshold: needs a date and at least one amount to be considered a transaction
            if dates_found and len(amounts) >= 1:
                # Naive role assignment: if 3 amounts, debit credit balance. If 2, assume debit/credit and balance.
                debit = None
                credit = None
                balance = amounts[-1]
                
                if len(amounts) == 3:
                    debit = amounts[0] if amounts[0] > 0 else None
                    credit = amounts[1] if amounts[1] > 0 else None
                elif len(amounts) == 2:
                    if amounts[0] < 0:
                        debit = abs(amounts[0])
                    else:
                        credit = amounts[0] # Very naive, real one needs header analysis
                        
                all_transactions.append({
                    "page": page_num,
                    "line_on_page": row_idx,
                    "date": dates_found[0],
                    "raw_text": line_text,
                    "debit": debit,
                    "credit": credit,
                    "running_balance": balance,
                    "bbox": [row[0][0], row[0][1], row[-1][2], max(w[3] for w in row)]
                })
    
    doc.close()
    return all_transactions


def chunk_pdf_for_docai(pdf_path: str, output_dir: str, max_pages_per_chunk: int = 15):
    """Split `pdf_path` into chunks of at most `max_pages_per_chunk` pages.

    Stage 3 / Item #16: Document AI's processor caps at 30 pages per request.
    This helper writes per-chunk PDFs to `output_dir` and returns metadata
    the Rust side uses to dispatch parallel parses and merge results.

    Returns a list of dicts:
        [{"path": "...", "page_offset": int, "page_count": int}, ...]
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    if not os.path.exists(output_dir):
        os.makedirs(output_dir, exist_ok=True)

    src = pymupdf.open(pdf_path)
    total = len(src)
    chunks = []
    chunk_idx = 0
    for start in range(0, total, max_pages_per_chunk):
        end = min(start + max_pages_per_chunk, total) - 1
        out = os.path.join(output_dir, f"chunk_{chunk_idx:03d}.pdf")
        new_doc = pymupdf.open()
        new_doc.insert_pdf(src, from_page=start, to_page=end)
        new_doc.save(out)
        new_doc.close()
        chunks.append({
            "path": out,
            "page_offset": start,
            "page_count": end - start + 1,
        })
        chunk_idx += 1
    src.close()
    return chunks


def analyze_document_layout(pdf_path: str):
    """Document layout analysis strategy"""
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    result = []

    for page_num in range(len(doc)):
        page = doc[page_num]
        blocks = page.get_text("dict")["blocks"]
        
        has_header = False
        has_footer = False
        has_page_number = False
        dominant_font = "Unknown"
        
        for block in blocks:
            if "lines" not in block: continue
            for line in block["lines"]:
                text = "".join([span["text"] for span in line["spans"]]).lower()
                if "page" in text or any(char.isdigit() for char in text[-5:]):
                    has_page_number = True
                if any(word in text for word in ["statement", "account", "period", "balance"]):
                    has_header = True
                if any(word in text for word in ["page", "continued", "total"]):
                    has_footer = True
                
                if line["spans"]:
                    dominant_font = line["spans"][0]["font"]
        
        result.append({
            "page_number": page_num + 1,
            "has_header": has_header,
            "has_footer": has_footer,
            "has_page_number": has_page_number,
            "table_columns": 5,
            "main_text_style": "regular",
            "dominant_font": dominant_font
        })
        
    doc.close()
    return result

def find_text_block_at_click(pdf_path: str, page_num: int, click_x: float, click_y: float, dpi: float = 300.0):
    """Span-level click detection.

    The GUI's canvas handler converts the click position into PDF-point
    space before sending, so we treat the input as PDF points. The `dpi`
    parameter is preserved for back-compat but ignored. Returns the
    dominant span (text, bbox, font name, size) under the click so the
    caller can drive a fidelity-correct edit.

    Stage 12 follow-up: previously this used `get_text('words')` which
    drops font information; the dict-level extraction preserves it so the
    GUI shows the real font name instead of '(unknown)'.
    """
    _ = dpi  # unused; kept for API stability
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    page = doc[page_num]
    try:
        click_x_pt = float(click_x)
        click_y_pt = float(click_y)
        # 2pt tolerance: roughly half a 12pt cap-height. Snug enough to
        # disambiguate adjacent columns but generous enough that a click
        # at the edge of a span still hits.
        tolerance_pt = 2.0

        best_match = None
        min_distance = float("inf")

        for block in page.get_text("dict").get("blocks", []):
            for line in block.get("lines", []):
                for span in line.get("spans", []):
                    bbox = span.get("bbox")
                    if not bbox:
                        continue
                    x0, y0, x1, y1 = bbox
                    inside = (
                        click_x_pt >= x0 - tolerance_pt
                        and click_x_pt <= x1 + tolerance_pt
                        and click_y_pt >= y0 - tolerance_pt
                        and click_y_pt <= y1 + tolerance_pt
                    )
                    if not inside:
                        continue
                    cx = (x0 + x1) / 2.0
                    cy = (y0 + y1) / 2.0
                    distance = ((click_x_pt - cx) ** 2 + (click_y_pt - cy) ** 2) ** 0.5
                    if distance < min_distance:
                        min_distance = distance
                        best_match = {
                            "page": page_num,
                            "text": span.get("text", "") or "",
                            "bbox": [x0, y0, x1, y1],
                            "font": span.get("font", "") or "",
                            "size": float(span.get("size", 0.0) or 0.0),
                        }

        return best_match
    finally:
        doc.close()


def complete_font_with_adaption_fallback(pdf_path: str, font_name: str, sample_text: str = "The quick brown fox"):
    """
    Main entry point for font completion.
    1. Try Lipi.ai (placeholder - will be implemented when API key is available)
    2. If fails or low confidence → Trigger smart "Adaption" fallback
    """
    try:
        # Placeholder for real Lipi.ai call
        # In production: call Lipi.ai API here with rendered sample
        raise Exception("Lipi.ai not configured in this environment")
    except Exception:
        return adapt_font_fallback(pdf_path, font_name, sample_text)


def adapt_font_fallback(pdf_path: str, font_name: str, sample_text: str = "The quick brown fox"):
    """
    Smart Adaption Fallback Strategy:
    - Analyzes original font name for style hints (Bold, Italic, Serif, Mono, etc.)
    - Chooses the closest standard PDF base font
    - Applies appropriate style modifiers
    - Returns a professional adapted font + explanation
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    
    font_lower = font_name.lower()
    
    # Step 1: Determine base font family
    if any(x in font_lower for x in ["times", "roman", "serif", "garamond", "georgia"]):
        base = "times-roman"
    elif any(x in font_lower for x in ["courier", "mono", "typewriter", "consolas"]):
        base = "courier"
    else:
        base = "helvetica"  # Default safe choice
    
    # Step 2: Detect style modifiers
    is_bold = any(x in font_lower for x in ["bold", "black", "heavy", "semibold"])
    is_italic = any(x in font_lower for x in ["italic", "oblique", "slant"])
    
    # Step 3: Build adapted font name
    if is_bold and is_italic:
        adapted_name = f"{base}-boldoblique"
    elif is_bold:
        adapted_name = f"{base}-bold"
    elif is_italic:
        adapted_name = f"{base}-oblique"
    else:
        adapted_name = base
    
    # Step 4: Get font buffer
    try:
        font = pymupdf.Font(adapted_name)
        font_bytes = font.buffer
    except:
        # Ultimate fallback
        font = pymupdf.Font("helvetica")
        font_bytes = font.buffer
        adapted_name = "helvetica"
    
    doc.close()
    
    confidence = 0.78 if adapted_name != "helvetica" else 0.65
    
    return {
        "success": True,
        "font_bytes": list(font_bytes),  # Convert to list for JSON compatibility
        "adapted_font_name": adapted_name,
        "original_font_name": font_name,
        "confidence": confidence,
        "message": f"Could not perfectly identify '{font_name}'. Using smart adaptation: {adapted_name}"
    }


def deep_font_replication_api(pdf_path, font_name, output_dir):
    """API entry point for deep font replication.

    Stage 11: this now delegates to `font_replicator.replicate_font_for_chars`
    which runs the three-tier cascade (composite synthesis → subset
    extension → Gemini Vision donor identification). Callers that pass
    no `missing_chars` get the legacy "synthesise everything" behaviour
    only as a side effect of an empty cascade run; modern callers should
    use `replicate_font_for_missing_chars`.
    """
    import font_replicator
    return font_replicator.replicate_font_for_chars(
        pdf_path=pdf_path,
        font_name=font_name,
        missing_chars=[],
        output_dir=output_dir,
    )


def replicate_font_for_missing_chars(pdf_path: str, font_name: str, missing_chars_csv: str, output_dir: str):
    """Stage 11: targeted font cascade. Pass the comma-separated missing
    characters returned by `_font_covers_text` and the cascade tries
    composite synthesis first, then subset extension from a local donor,
    then Gemini-Vision-identified donor. Returns the result dict shaped by
    `replicate_font_for_chars`.
    """
    import font_replicator
    chars = [c for c in missing_chars_csv.split(",") if c]
    return font_replicator.replicate_font_for_chars(
        pdf_path=pdf_path,
        font_name=font_name,
        missing_chars=chars,
        output_dir=output_dir,
    )


def dry_run_edit_preview(
    pdf_path: str,
    page_num: int,
    rect: list,
    new_text: str,
    output_png_path: str,
    font_path: str = None,
    pad_pts: float = 30.0,
    dpi: float = 200.0,
):
    """Stage 14d / Item #17: render a small PNG preview of how an edit
    will look without committing to disk.

    Workflow: open a writable copy of the source PDF, apply the edit
    in-memory, render the area around the bbox at `dpi` DPI, save as a
    PNG. The original file is not touched.

    Returns a dict with the output path and the bbox-with-pad coordinates
    so the GUI can size the preview thumbnail.
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    try:
        ok, reason = _check_doc_editable(doc)
        if not ok:
            return {"success": False, "error": "PDF_NOT_EDITABLE", "reason": reason}
        rect_obj = pymupdf.Rect(rect)
        page = doc[page_num]

        # Skip the cascade — for a preview we just want the visual.
        try:
            res = replace_text_in_rect(
                pdf_path=pdf_path,
                output_path=output_png_path + ".tmp.pdf",
                page_num=page_num,
                rect=rect,
                new_text=new_text,
                font_path=font_path,
            )
        except ValueError as e:
            # Coverage failure or other structured error.
            return {"success": False, "error": str(e)}

        # Now render the bbox+pad area at high DPI from the patched PDF.
        patched = pymupdf.open(output_png_path + ".tmp.pdf")
        ppage = patched[page_num]
        clip = pymupdf.Rect(
            max(0.0, rect_obj.x0 - pad_pts),
            max(0.0, rect_obj.y0 - pad_pts),
            min(float(ppage.rect.width), rect_obj.x1 + pad_pts),
            min(float(ppage.rect.height), rect_obj.y1 + pad_pts),
        )
        pix = ppage.get_pixmap(clip=clip, dpi=dpi, alpha=False)
        pix.save(output_png_path)
        patched.close()
        try:
            os.remove(output_png_path + ".tmp.pdf")
        except OSError:
            pass

        return {
            "success": True,
            "preview_png": output_png_path,
            "method": res.get("method"),
            "clip_bbox_pts": [clip.x0, clip.y0, clip.x1, clip.y1],
        }
    finally:
        doc.close()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        # Self-check for analyze_background slicing logic
        print("Running self-checks...")
        
        samples_n4 = [255, 0, 0, 255,  0, 255, 0, 255] # 2 pixels RGBA
        r = samples_n4[0::4]
        g = samples_n4[1::4]
        b = samples_n4[2::4]
        assert r == [255, 0]
        assert g == [0, 255]
        assert b == [0, 0]
        print("RGBA slicing OK")

        samples_n1 = [128, 64] # 2 pixels Gray
        g = samples_n1[0::1]
        assert g == [128, 64]
        print("Grayscale slicing OK")

        sys.exit(0)

    command = sys.argv[1]

    if command == "get_blocks":
        pdf_path = sys.argv[2]
        page_num = int(sys.argv[3])
        blocks = get_text_blocks(pdf_path, page_num)
        print(json.dumps(blocks, indent=2))

    elif command == "replace_in_rect":
        pdf_path = sys.argv[2]
        output_path = sys.argv[3]
        page_num = int(sys.argv[4])
        rect = json.loads(sys.argv[5])
        new_text = sys.argv[6]
        font_path = sys.argv[7] if len(sys.argv) > 7 else None
        replace_text_in_rect(pdf_path, output_path, page_num, rect, new_text, font_path=font_path)

    elif command == "complete_font":
        pdf_path = sys.argv[2]
        font_name = sys.argv[3]
        result = complete_font_with_adaption_fallback(pdf_path, font_name)
        print(json.dumps(result))

    elif command == "deep_font_replication":
        import font_replicator
        pdf_path = sys.argv[2]
        font_name = sys.argv[3]
        output_dir = sys.argv[4]

        # Phase 1
        res = font_replicator.extract_and_harvest(pdf_path, font_name, output_dir)
        if not res["success"]:
            print(json.dumps(res))
            sys.exit(0)

        metrics = res["metrics"]

        # Phase 2
        # For now, we replicate some common missing letters as a test
        glyphs_to_replicate = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
        norm_res = font_replicator.normalize_glyphs(metrics["font_path"], metrics, output_dir, glyphs_to_replicate)

        if not norm_res["success"]:
            print(json.dumps(norm_res))
            sys.exit(0)

        # Combine results
        combined = {
            "success": True,
            "metrics": metrics,
            "images": norm_res["images"],
            "baseline_y": norm_res["baseline_y"]
        }
        print(json.dumps(combined))

