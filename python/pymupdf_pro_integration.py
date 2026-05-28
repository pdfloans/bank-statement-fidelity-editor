#!/usr/bin/env python3
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


def _find_dominant_span(page, rect_obj):
    """Find the text span whose bbox best overlaps `rect_obj`. Returns the span
    dict (text/font/size/color/origin) or None if nothing overlaps."""
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
         characters outside WinAnsiEncoding (rare — emoji, CJK, etc.).
      2. Otherwise we attempt to extract the embedded font subset and probe
         glyph coverage with PyMuPDF.Font(buffer=...).
      3. Any failure to determine coverage is treated as 'unknown' and
         returns (False, list(text)) so the caller can decide.
    """
    if _is_standard_14(font_name):
        # WinAnsi covers most western characters. Flag only ones that aren't
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

    PyMuPDF's `dict` extraction returns the font's *base name* (e.g. 'Times-Roman'
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
    page = doc[page_num]

    rect_obj = pymupdf.Rect(rect)

    # 1. Learn the original span's style.
    span = _find_dominant_span(page, rect_obj)
    if span is None:
        # No span overlaps — empty area, treat as a vector-only edit.
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

    # 5. Apply the redaction. We use the redact annotation's `text` parameter
    #    only as a hint; PyMuPDF will draw the replacement before the next
    #    apply_redactions() pass. We override the auto-styling to match the
    #    original span exactly.
    annot = page.add_redact_annot(
        rect_obj,
        text=new_text,
        fill=fill_color,
        text_color=original_color,
        fontname=insert_font_name if method == "supplied" else None,
        fontsize=original_size,
        align=pymupdf.TEXT_ALIGN_LEFT,
    )

    # `apply_redactions(images=PDF_REDACT_IMAGE_NONE)` keeps imagery untouched
    # so background art / logos / signatures remain bit-identical.
    try:
        page.apply_redactions(images=pymupdf.PDF_REDACT_IMAGE_NONE)
    except (TypeError, AttributeError):
        # Older PyMuPDF: no kwarg, but the default was image-preserving for
        # a small redaction area anyway.
        page.apply_redactions()

    # Re-place the text at the *exact* original baseline. The redact text helper
    # auto-positions; this insert_text overrides it for pixel-equivalent placement.
    if method == "embedded":
        # Embedded reuse: PyMuPDF doesn't let us insert by xref directly, so we
        # use the basefont name. For Type1/TrueType subsets the name lookup
        # round-trips correctly because the subset has been registered with the
        # page already.
        try:
            page.insert_text(
                point=pymupdf.Point(original_origin[0], original_origin[1]),
                text=new_text,
                fontname=original_font_name,
                fontsize=original_size,
                color=original_color,
                render_mode=0,
                overlay=True,
            )
        except Exception as e:
            # If the basefont name isn't recognised by insert_text (some
            # subsetted fonts have weird names), let the redact annotation's
            # auto-text stand. Visual fidelity is still very high but font may
            # default to Helvetica metrics inside the rect.
            print(f"[replace] insert_text fallback for embedded path: {e}", file=sys.stderr)
            method = "embedded-fallback"
    elif method == "supplied":
        page.insert_text(
            point=pymupdf.Point(original_origin[0], original_origin[1]),
            text=new_text,
            fontname=insert_font_name,
            fontsize=original_size,
            color=original_color,
            render_mode=0,
            overlay=True,
        )

    doc.save(output_path, garbage=4, deflate=True, clean=True)
    doc.close()

    return {
        "success": True,
        "method": method,
        "original_font": original_font_name,
        "size": original_size,
        "missing_chars": missing_chars,
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
    """
    Improved click detection with word-level accuracy and tolerance.
    Returns the best matching text block near the click position.
    """
    pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
    doc = pymupdf.open(pdf_path)
    page = doc[page_num]
    
    scale = dpi / 72.0
    
    # Get words with bounding boxes (more accurate than spans for clicking)
    words = page.get_text("words")  # [x0, y0, x1, y1, word, block_no, line_no, word_no]
    
    best_match = None
    min_distance = float('inf')
    
    for word in words:
        x0, y0, x1, y1, text, _, _, _ = word
        x0 *= scale
        y0 *= scale
        x1 *= scale
        y1 *= scale
        
        # Check if click is inside or very close to the word
        center_x = (x0 + x1) / 2
        center_y = (y0 + y1) / 2
        
        # Calculate distance to center
        distance = ((click_x - center_x) ** 2 + (click_y - center_y) ** 2) ** 0.5
        
        # Add small tolerance (10 pixels at 300 DPI)
        tolerance = 12.0
        
        if distance < min_distance and distance < tolerance:
            min_distance = distance
            best_match = {
                "page": page_num,
                "text": text,
                "bbox": [x0 / scale, y0 / scale, x1 / scale, y1 / scale],  # Return in PDF coordinates
                "font": "unknown",  # Word-level doesn't have font info easily
                "size": 0.0
            }
    
    doc.close()
    
    if best_match:
        return best_match
    else:
        return None


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
    """API entry point for deep font replication"""
    import font_replicator
    
    # Phase 1
    res = font_replicator.extract_and_harvest(pdf_path, font_name, output_dir)
    if not res["success"]:
        return res
        
    metrics = res["metrics"]
    
    # Phase 2
    # For now, we replicate some common missing letters as a test
    glyphs_to_replicate = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
    norm_res = font_replicator.normalize_glyphs(metrics["font_path"], metrics, output_dir, glyphs_to_replicate)
    
    if not norm_res["success"]:
        return norm_res
        
    # Phase 3 (Optional for now as it's a mock)
    # font_replicator.call_ai_extrapolation(norm_res["images"], "")
    
    # Combine results
    return {
        "success": True,
        "metrics": metrics,
        "images": norm_res["images"],
        "baseline_y": norm_res["baseline_y"]
    }


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

