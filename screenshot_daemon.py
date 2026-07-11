import time
import os
from PIL import ImageGrab

OUTPUT_FILE = "desktop_screenshot.png"
INTERVAL = 3

print("==================================================")
print("             Visual Bridge Daemon                 ")
print("==================================================")
print(f"Capturing screen every {INTERVAL} seconds to: {OUTPUT_FILE}")
print("Leave this terminal open/minimized.")
print("Press Ctrl+C to stop.")
print("==================================================\n")

try:
    while True:
        try:
            # Grab the entire screen (across all monitors)
            screenshot = ImageGrab.grab(all_screens=True)
            # Save the image, overwriting the previous one
            screenshot.save(OUTPUT_FILE, format="PNG")
            print(f"[{time.strftime('%X')}] ✓ Screen captured")
        except Exception as e:
            print(f"[{time.strftime('%X')}] ✗ Failed to take screenshot: {e}")
            
        time.sleep(INTERVAL)
except KeyboardInterrupt:
    print("\nVisual Bridge stopped.")
