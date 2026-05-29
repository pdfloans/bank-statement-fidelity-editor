#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
Stage 11 - Font creation cascade.

When the binary editor reports FONT_COVERAGE_INSUFFICIENT for a set of
characters, this module is asked to extend the original font's embedded
subset so those characters become renderable. The cascade tries, in order:

  1. Composite glyph synthesis (Item #10).
       For precomposed characters whose decomposition references glyphs
       that already exist in the subset (e.g. NFD: e + U+0301 acute -> e),
       build the precomposed glyph from base + diacritic via fontTools'
       glyph composite mechanism. No external dependency, no visual drift.

  2. Subset extension via fontTools (Item #7).
       Pick a donor font with matching metrics from the local cache,
       copy only the still-missing glyphs into the original subset's
       cmap, glyf, hmtx, and OS/2 tables, re-embed in the document.
       Visual drift is small for shapes that aren't in the original
       (the donor's outlines, hopefully a near-match typeface).

  3. AI font identification via Gemini Vision (Item #9).
       Rasterise the original glyphs at 600 DPI and ask Gemini Vision
       which typeface it most resembles. Fetch from a curated Google
       Fonts cache and use as the donor for Tier 2.

If all three tiers fail or no progress is made, return a structured
failure with `still_missing`.

The cache lives at `cache/fonts/` and is populated lazily on first use.
The cascade entry point is `replicate_font_for_chars`.
"""

import io
import json
import os
import shutil
import sys
import tempfile
import unicodedata
from typing import Dict, List, Optional, Tuple


# Public alias kept for back-compat with old callers.
def deep_font_replication_api(pdf_path, font_name, output_dir):
    return replicate_font_for_chars(
        pdf_path=pdf_path,
        font_name=font_name,
        missing_chars=[],
        output_dir=output_dir,
    )


# ---------------------------------------------------------------------------
# Cache discovery
# ---------------------------------------------------------------------------

_DEFAULT_CACHE_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "cache", "fonts"
)


def _cache_dir() -> str:
    return os.environ.get("FONT_CACHE_DIR", _DEFAULT_CACHE_DIR)


def _ensure_cache_dir() -> str:
    d = _cache_dir()
    os.makedirs(d, exist_ok=True)
    return d


# ---------------------------------------------------------------------------
# Tier 1: composite glyph synthesis
# ---------------------------------------------------------------------------

def _decompose_to_components(ch: str) -> List[str]:
    """Return the NFD decomposition of `ch` as a list of single-codepoint
    strings. For precomposed letters this typically returns
    [base, combining_mark]. Returns [ch] if the char has no decomposition.
    """
    decomp = unicodedata.normalize("NFD", ch)
    if decomp == ch:
        return [ch]
    return list(decomp)


def _try_composite_synthesis(
    original_font_path: str,
    output_path: str,
    missing_chars: List[str],
) -> Tuple[List[str], List[str]]:
    """Try to build each missing precomposed character from existing
    components in the original subset. Returns (synthesised, still_missing).

    Implementation: for each missing precomposed glyph, decompose to NFD
    and check whether every component is already in the cmap. If yes, add
    a `glyf` composite entry that references those component glyph IDs at
    the standard combining offset. Save under `output_path`.
    """
    try:
        from fontTools.ttLib import TTFont
        from fontTools.ttLib.tables._g_l_y_f import Glyph, GlyphComponent
    except Exception as e:
        print(f"[fr] composite tier needs fontTools: {e}", file=sys.stderr)
        return ([], list(missing_chars))

    if not missing_chars:
        return ([], [])

    try:
        font = TTFont(original_font_path)
    except Exception as e:
        print(f"[fr] composite tier - cannot open original: {e}", file=sys.stderr)
        return ([], list(missing_chars))

    # Subsetted PDF fonts often strip non-essential tables. Bail when the
    # required ones aren't there; the next tier will handle it.
    required = ("cmap", "glyf", "hmtx")
    missing_tables = [t for t in required if t not in font]
    if missing_tables:
        print(
            f"[fr] composite tier skipped: subset is missing tables {missing_tables}",
            file=sys.stderr,
        )
        return ([], list(missing_chars))

    cmap = font.getBestCmap()
    if "glyf" not in font or "hmtx" not in font:
        # Not a TrueType-flavoured font (likely CFF/OTF). Composite via
        # `glyf` does not apply here; fall through to subset extension.
        return ([], list(missing_chars))

    glyf = font["glyf"]
    hmtx = font["hmtx"]

    synthesised = []
    still_missing = []

    for ch in missing_chars:
        cp = ord(ch)
        if cp in cmap:
            # Already present somehow - skip.
            synthesised.append(ch)
            continue
        components = _decompose_to_components(ch)
        if len(components) < 2:
            still_missing.append(ch)
            continue

        # Verify every component is in the original cmap.
        component_glyph_names = []
        all_present = True
        for comp in components:
            comp_cp = ord(comp)
            if comp_cp not in cmap:
                all_present = False
                break
            component_glyph_names.append(cmap[comp_cp])
        if not all_present:
            still_missing.append(ch)
            continue

        # Build the composite glyph.
        # Diacritic positioning: rough centring on top of the base. A
        # production implementation would consult MARK / MKMK GPOS rules;
        # this version uses a simple stack at the base's advance / 2.
        new_name = f"uni{cp:04X}"
        # Avoid collisions with existing glyph names.
        if new_name in font.getGlyphOrder():
            new_name = f"{new_name}_synth"

        composite = Glyph()
        composite.numberOfContours = -1
        composite.components = []

        base_name = component_glyph_names[0]
        base_glyph = glyf[base_name]
        base_w = hmtx.metrics.get(base_name, (1000, 0))[0]

        for idx, gname in enumerate(component_glyph_names):
            comp = GlyphComponent()
            comp.glyphName = gname
            comp.flags = 0
            if idx == 0:
                comp.x, comp.y = 0, 0
            else:
                # Centre the mark over the base. Real fonts use anchors;
                # we approximate by centring horizontally.
                mark_w = hmtx.metrics.get(gname, (0, 0))[0]
                comp.x = (base_w - mark_w) // 2
                comp.y = 0
            composite.components.append(comp)

        # Add to glyf.
        glyf[new_name] = composite
        # Add to cmap.
        for table in font["cmap"].tables:
            if table.isUnicode():
                table.cmap[cp] = new_name
        # Add to hmtx using base width as advance.
        hmtx.metrics[new_name] = (base_w, 0)
        # Add to glyph order.
        order = font.getGlyphOrder()
        if new_name not in order:
            font.setGlyphOrder(order + [new_name])

        synthesised.append(ch)

    if synthesised:
        try:
            font.save(output_path)
            print(f"[fr] composite tier synthesised {len(synthesised)} glyph(s)", file=sys.stderr)
        except Exception as e:
            print(f"[fr] composite tier save failed: {e}", file=sys.stderr)
            return ([], list(missing_chars))

    return (synthesised, still_missing)


# ---------------------------------------------------------------------------
# Tier 2: subset extension from a donor font
# ---------------------------------------------------------------------------

def _try_subset_extension(
    original_font_path: str,
    donor_font_path: str,
    output_path: str,
    missing_chars: List[str],
) -> Tuple[List[str], List[str]]:
    """Copy missing glyphs from `donor_font_path` into the original subset.
    Returns (extended_with, still_missing).
    """
    try:
        from fontTools.ttLib import TTFont
    except Exception as e:
        print(f"[fr] subset extension needs fontTools: {e}", file=sys.stderr)
        return ([], list(missing_chars))

    if not missing_chars or not os.path.isfile(donor_font_path):
        return ([], list(missing_chars))

    try:
        original = TTFont(original_font_path)
        donor = TTFont(donor_font_path)
    except Exception as e:
        print(f"[fr] subset extension - cannot open: {e}", file=sys.stderr)
        return ([], list(missing_chars))

    # Original or donor without the required tables → nothing the cascade
    # can do. Most subsetted PDF fonts fall in this bucket.
    required = ("cmap", "glyf", "hmtx", "head")
    missing_orig = [t for t in required if t not in original]
    missing_donor = [t for t in required if t not in donor]
    if missing_orig:
        print(
            f"[fr] subset extension skipped: original missing tables {missing_orig}",
            file=sys.stderr,
        )
        return ([], list(missing_chars))
    if missing_donor:
        print(
            f"[fr] subset extension skipped: donor missing tables {missing_donor}",
            file=sys.stderr,
        )
        return ([], list(missing_chars))

    donor_cmap = donor.getBestCmap()
    original_cmap = original.getBestCmap()

    extended = []
    still_missing = []

    if "glyf" not in original or "glyf" not in donor:
        # CFF or hybrid; we can still extend cmap to map missing codepoints
        # to glyphs that exist in the donor by name only. Keep it simple:
        # report all-missing and let Tier 3 take over.
        return ([], list(missing_chars))

    original_glyf = original["glyf"]
    original_hmtx = original["hmtx"]
    donor_glyf = donor["glyf"]
    donor_hmtx = donor["hmtx"]

    # Scale factor in case donor and original have different upem.
    upem_orig = float(original["head"].unitsPerEm)
    upem_donor = float(donor["head"].unitsPerEm)
    scale = upem_orig / upem_donor if upem_donor else 1.0

    glyph_order = list(original.getGlyphOrder())

    for ch in missing_chars:
        cp = ord(ch)
        if cp in original_cmap:
            extended.append(ch)
            continue
        if cp not in donor_cmap:
            still_missing.append(ch)
            continue

        donor_glyph_name = donor_cmap[cp]
        donor_glyph = donor_glyf.get(donor_glyph_name)
        if donor_glyph is None:
            still_missing.append(ch)
            continue

        new_name = f"uni{cp:04X}_donor"
        # Avoid name collisions.
        idx = 0
        while new_name in glyph_order:
            idx += 1
            new_name = f"uni{cp:04X}_donor{idx}"

        # Copy the glyph data. fontTools deep-copies via XML-roundtrip
        # path; we use the simpler TTGlyphPen for safety with composite
        # donor glyphs.
        try:
            from fontTools.pens.ttGlyphPen import TTGlyphPen
            pen = TTGlyphPen(donor.getGlyphSet())
            donor.getGlyphSet()[donor_glyph_name].draw(pen)
            new_glyph = pen.glyph()
        except Exception as e:
            print(f"[fr] glyph copy failed for {ch!r}: {e}", file=sys.stderr)
            still_missing.append(ch)
            continue

        # Insert.
        original_glyf[new_name] = new_glyph
        glyph_order.append(new_name)
        donor_w = donor_hmtx.metrics.get(donor_glyph_name, (1000, 0))[0]
        original_hmtx.metrics[new_name] = (int(round(donor_w * scale)), 0)

        for table in original["cmap"].tables:
            if table.isUnicode():
                table.cmap[cp] = new_name

        extended.append(ch)

    original.setGlyphOrder(glyph_order)

    if extended:
        try:
            original.save(output_path)
            print(f"[fr] subset extension copied {len(extended)} glyph(s) from donor", file=sys.stderr)
        except Exception as e:
            print(f"[fr] subset extension save failed: {e}", file=sys.stderr)
            return ([], list(missing_chars))

    return (extended, still_missing)


# ---------------------------------------------------------------------------
# Tier 3: Gemini Vision identification
# ---------------------------------------------------------------------------

def _identify_typeface_via_gemini(font_name: str, glyph_image_path: str) -> Optional[str]:
    """Ask Gemini Vision which typeface the rasterised glyphs match.
    Returns the donor's local cache path if a known font is identified,
    None otherwise.

    The cache contains a manifest at `cache/fonts/manifest.json` mapping
    canonical typeface names to local TTF paths. Recommended seed:

        {
          "Arial":              "arial.ttf",
          "Helvetica":          "Helvetica.ttf",
          "Times New Roman":    "times.ttf",
          "Roboto":             "Roboto-Regular.ttf",
          "Open Sans":          "OpenSans-Regular.ttf",
          "Noto Sans":          "NotoSans-Regular.ttf",
          "Source Sans Pro":    "SourceSansPro-Regular.ttf",
          "Inter":              "Inter-Regular.ttf"
        }
    """
    cache = _ensure_cache_dir()
    manifest_path = os.path.join(cache, "manifest.json")
    if not os.path.isfile(manifest_path):
        print(f"[fr] no font manifest at {manifest_path}; Tier 3 skipped", file=sys.stderr)
        return None

    api_key = os.environ.get("GEMINI_API_KEY")
    if not api_key:
        print("[fr] GEMINI_API_KEY not set; Tier 3 skipped", file=sys.stderr)
        return None

    try:
        with io.open(manifest_path, "r", encoding="utf-8") as f:
            manifest: Dict[str, str] = json.load(f)
    except Exception as e:
        print(f"[fr] manifest load failed: {e}", file=sys.stderr)
        return None

    if not os.path.isfile(glyph_image_path):
        return None

    try:
        import base64
        import urllib.request
        with io.open(glyph_image_path, "rb") as f:
            img_b64 = base64.b64encode(f.read()).decode("ascii")

        candidates = sorted(manifest.keys())
        prompt = (
            "Identify the typeface in this image. The known typeface is "
            f"\"{font_name}\". Choose the BEST single match from this list:\n"
            + "\n".join("- " + c for c in candidates)
            + "\n\nReturn only the chosen name with no other text."
        )
        body = {
            "contents": [{
                "parts": [
                    {"text": prompt},
                    {"inlineData": {"mimeType": "image/png", "data": img_b64}}
                ]
            }],
        }
        url = (
            "https://generativelanguage.googleapis.com/v1beta/models/"
            "gemini-2.5-flash:generateContent?key=" + api_key
        )
        req = urllib.request.Request(
            url,
            data=json.dumps(body).encode("utf-8"),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            payload = json.loads(resp.read().decode("utf-8"))
        text = (
            payload.get("candidates", [{}])[0]
            .get("content", {})
            .get("parts", [{}])[0]
            .get("text", "")
            .strip()
        )
    except Exception as e:
        print(f"[fr] Gemini font ID failed: {e}", file=sys.stderr)
        return None

    if not text:
        return None
    # Find the first candidate name that appears in Gemini's response.
    pick = None
    for c in candidates:
        if c.lower() in text.lower():
            pick = c
            break
    if pick is None:
        return None
    rel = manifest.get(pick)
    if not rel:
        return None
    donor_path = os.path.join(cache, rel)
    if not os.path.isfile(donor_path):
        print(f"[fr] manifest pointed to missing file {donor_path}", file=sys.stderr)
        return None
    print(f"[fr] Gemini identified typeface as {pick}", file=sys.stderr)
    return donor_path


def _rasterise_subset(font_path: str, output_path: str, sample_chars: str = "ABCDEFGabcdefg012345") -> bool:
    """Render a row of `sample_chars` from `font_path` to a PNG. Used for
    Gemini Vision typeface ID (Tier 3)."""
    try:
        from PIL import Image, ImageDraw, ImageFont
    except Exception as e:
        print(f"[fr] PIL needed to rasterise: {e}", file=sys.stderr)
        return False
    try:
        size_px = 96
        font = ImageFont.truetype(font_path, size=size_px)
        # Approximate canvas; over-sized then crop.
        canvas = Image.new("L", (len(sample_chars) * size_px, size_px * 2), color=255)
        draw = ImageDraw.Draw(canvas)
        draw.text((10, 5), sample_chars, font=font, fill=0)
        bbox = canvas.getbbox()
        if bbox:
            canvas = canvas.crop(bbox)
        canvas.save(output_path, "PNG")
        return True
    except Exception as e:
        print(f"[fr] rasterise failed: {e}", file=sys.stderr)
        return False


# ---------------------------------------------------------------------------
# Cascade entry point
# ---------------------------------------------------------------------------

def replicate_font_for_chars(
    pdf_path: str,
    font_name: str,
    missing_chars: List[str],
    output_dir: str,
) -> Dict:
    """Top-level cascade. Returns:

        {
          "success": bool,
          "extended_font_path": str | None,    # path to the new TTF/OTF
          "synthesised": [chars done by Tier 1],
          "donor_extended": [chars done by Tier 2],
          "ai_extended": [chars done by Tier 3],
          "still_missing": [chars not covered by any tier],
          "tiers_used": ["composite" | "subset_extension" | "gemini_vision"]
        }
    """
    os.makedirs(output_dir, exist_ok=True)
    if not missing_chars:
        return {
            "success": True,
            "extended_font_path": None,
            "synthesised": [],
            "donor_extended": [],
            "ai_extended": [],
            "still_missing": [],
            "tiers_used": [],
        }

    # 1. Extract the original font subset to disk.
    try:
        import pymupdf
        import pymupdf.pro
        # Reuse the integration's key (best-effort; the caller may have
        # already unlocked).
        try:
            from python.pymupdf_pro_integration import PYMUPDF_PRO_KEY  # type: ignore
            pymupdf.pro.unlock(PYMUPDF_PRO_KEY)
        except Exception:
            pass
        doc = pymupdf.open(pdf_path)
    except Exception as e:
        return {
            "success": False,
            "error": f"cannot open PDF: {e}",
            "still_missing": list(missing_chars),
        }

    original_font_path = None
    try:
        for page in doc:
            for f in page.get_fonts(full=True):
                xref, ext, ftype, basefont, alias, encoding = (list(f) + [None]*6)[:6]
                if not basefont:
                    continue
                if font_name.lower() in (basefont or "").lower() or font_name.lower() in (alias or "").lower():
                    info = doc.extract_font(xref)
                    content = None
                    if isinstance(info, dict):
                        content = info.get("content")
                        ext = info.get("ext", ext or "ttf")
                    elif isinstance(info, (tuple, list)):
                        for item in reversed(info):
                            if isinstance(item, (bytes, bytearray)) and item:
                                content = bytes(item)
                                break
                    if content:
                        original_font_path = os.path.join(output_dir, f"original_subset.{ext or 'ttf'}")
                        with io.open(original_font_path, "wb") as out:
                            out.write(content)
                        break
            if original_font_path:
                break
    finally:
        doc.close()

    if not original_font_path:
        return {
            "success": False,
            "error": f"could not extract embedded font subset for {font_name!r}",
            "still_missing": list(missing_chars),
        }

    tiers_used = []
    remaining = list(missing_chars)

    # Tier 1: composite synthesis.
    composite_out = os.path.join(output_dir, "extended_after_composite.ttf")
    synth, remaining = _try_composite_synthesis(original_font_path, composite_out, remaining)
    if synth:
        tiers_used.append("composite")
        # Future tiers operate on the extended version.
        original_font_path = composite_out

    # Tier 2: subset extension from a donor.
    donor_extended = []
    donor_path = _pick_local_donor(font_name)
    if donor_path and remaining:
        ext_out = os.path.join(output_dir, "extended_after_donor.ttf")
        donor_extended, remaining = _try_subset_extension(
            original_font_path, donor_path, ext_out, remaining
        )
        if donor_extended:
            tiers_used.append("subset_extension")
            original_font_path = ext_out

    # Tier 3: Gemini Vision typeface ID, then re-run Tier 2 with the
    # identified donor.
    ai_extended = []
    if remaining:
        sample_png = os.path.join(output_dir, "original_sample.png")
        if _rasterise_subset(original_font_path, sample_png):
            ai_donor = _identify_typeface_via_gemini(font_name, sample_png)
            if ai_donor:
                ext_out = os.path.join(output_dir, "extended_after_ai.ttf")
                ai_extended, remaining = _try_subset_extension(
                    original_font_path, ai_donor, ext_out, remaining
                )
                if ai_extended:
                    tiers_used.append("gemini_vision")
                    original_font_path = ext_out

    final_path = original_font_path if (synth or donor_extended or ai_extended) else None

    return {
        "success": not remaining,
        "extended_font_path": final_path,
        "synthesised": synth,
        "donor_extended": donor_extended,
        "ai_extended": ai_extended,
        "still_missing": remaining,
        "tiers_used": tiers_used,
    }


def _pick_local_donor(font_name: str) -> Optional[str]:
    """Pick a local cached donor whose name best matches `font_name` by
    case-insensitive substring. Falls back to None.
    """
    cache = _ensure_cache_dir()
    manifest_path = os.path.join(cache, "manifest.json")
    if not os.path.isfile(manifest_path):
        return None
    try:
        with io.open(manifest_path, "r", encoding="utf-8") as f:
            manifest = json.load(f)
    except Exception:
        return None
    needle = font_name.lower()
    # Strip subset prefix.
    if "+" in needle:
        needle = needle.split("+", 1)[1]
    # Direct match.
    for name, rel in manifest.items():
        if needle in name.lower() or name.lower() in needle:
            p = os.path.join(cache, rel)
            if os.path.isfile(p):
                return p
    return None


# ---------------------------------------------------------------------------
# CLI entry for testing.
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    if len(sys.argv) < 5:
        print(
            "Usage: python font_replicator.py <pdf> <font_name> <output_dir> <missing_chars_comma_separated>",
            file=sys.stderr,
        )
        sys.exit(1)
    res = replicate_font_for_chars(
        pdf_path=sys.argv[1],
        font_name=sys.argv[2],
        output_dir=sys.argv[3],
        missing_chars=sys.argv[4].split(","),
    )
    print(json.dumps(res, indent=2))
