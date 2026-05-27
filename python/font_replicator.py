import os
import sys
import json
import pymupdf
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

def extract_and_harvest(pdf_path, font_name, output_dir):
    """Phase 1: Extraction & Metadata Harvesting"""
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)

    doc = pymupdf.open(pdf_path)
    font_extracted = False
    font_path = None

    # Search for the font in the PDF
    for page in doc:
        fonts = page.get_fonts()
        for f in fonts:
            # f is (xref, ext, type, basefont, name, encoding)
            if font_name in f[3] or font_name in f[4]:
                xref = f[0]
                # Extract the font file
                font_info = doc.extract_font(xref)
                ext = font_info["ext"]
                content = font_info["content"]
                
                font_path = os.path.join(output_dir, f"extracted_subset.{ext}")
                with open(font_path, "wb") as fout:
                    fout.write(content)
                font_extracted = True
                break
        if font_extracted:
            break
    
    doc.close()

    if not font_extracted:
        return {"success": False, "error": f"Font {font_name} not found or could not be extracted"}

    # Harvest Metrics
    try:
        font = TTFont(font_path)
        upm = font['head'].unitsPerEm
        ascender = font['hhea'].ascent
        descender = font['hhea'].descent
        
        metrics = {
            "upm": upm,
            "ascender": ascender,
            "descender": descender,
            "font_path": font_path
        }
        return {"success": True, "metrics": metrics}
    except Exception as e:
        return {"success": False, "error": f"Failed to harvest metrics: {str(e)}"}

def normalize_glyphs(font_path, metrics, output_dir, glyphs_to_render):
    """Phase 2: Vector-to-Raster Normalization"""
    if not os.path.exists(output_dir):
        os.makedirs(output_dir)

    upm = metrics["upm"]
    ascender = metrics["ascender"]
    descender = metrics["descender"]
    
    # Total height we consider is ascender + abs(descender) or just UPM?
    # Usually we use UPM for the canvas size
    canvas_size = 1024 # Standard square canvas
    scale = canvas_size / upm
    
    # Baseline position: bottom of letters like 'H' is at the baseline.
    # Baseline is at 'ascender' units from the top in the font grid? 
    # Actually, ascender is distance from baseline to top. 
    # Descender is distance from baseline to bottom (usually negative).
    # Baseline in image = canvas_size * (ascender / (ascender - descender))?
    # No, if UPM is 1000, and ascender is 800, baseline is at 800 from bottom of the 1000-unit box.
    # So from top, baseline is at 200 (UPM - ascender).
    
    baseline_y = int((upm - ascender) * scale)
    
    rendered_images = []
    
    try:
        # We need a way to render the extracted font. 
        # Pillow can load .ttf
        pil_font = ImageFont.truetype(font_path, size=int(upm * scale))
        
        for char in glyphs_to_render:
            img = Image.new("L", (canvas_size, canvas_size), color=255) # White background
            draw = ImageDraw.Draw(img)
            
            # Draw the character. We need to align it to the baseline.
            # In Pillow, text drawing (x, y) is top-left.
            # We want the baseline of the text to be at baseline_y.
            # The 'ascent' of the rendered font should help.
            
            # Simplified: draw at (0, 0) and see where it lands, or use anchors.
            # Pillow 9.2.0+ supports anchors. 'ls' means left-baseline.
            draw.text((0, baseline_y), char, font=pil_font, fill=0, anchor="ls")
            
            char_path = os.path.join(output_dir, f"glyph_{ord(char)}.png")
            img.save(char_path)
            rendered_images.append({"char": char, "path": char_path})
            
        return {"success": True, "images": rendered_images, "baseline_y": baseline_y}
    except Exception as e:
        return {"success": False, "error": f"Failed to normalize glyphs: {str(e)}"}

def call_ai_extrapolation(images, api_key):
    """Phase 3: AI Extrapolation (Placeholder/Mock)"""
    # In a real scenario, this would send 'images' to an external API (Lipi.ai/FontDiffuser)
    # and receive back images for the missing letters.
    # For now, we mock this by just returning the same images or "simulating" missing ones.
    
    # Let's assume we want to generate 'g', 'y' if missing.
    # This is just a stub.
    return {"success": True, "message": "AI Extrapolation simulated"}

if __name__ == "__main__":
    # Test/CLI usage
    if len(sys.argv) < 2:
        print("Usage: python font_replicator.py <pdf_path> <font_name> <output_dir>")
        sys.exit(1)
        
    pdf_path = sys.argv[1]
    font_name = sys.argv[2]
    output_dir = sys.argv[3]
    
    result = extract_and_harvest(pdf_path, font_name, output_dir)
    if result["success"]:
        metrics = result["metrics"]
        print(f"Metrics: {json.dumps(metrics)}")
        
        # Test normalization with 'A' and 'B'
        norm_result = normalize_glyphs(metrics["font_path"], metrics, output_dir, "AB")
        if norm_result["success"]:
            print(f"Normalized: {json.dumps(norm_result['images'])}")
        else:
            print(f"Normalization failed: {norm_result['error']}")
    else:
        print(f"Extraction failed: {result['error']}")
