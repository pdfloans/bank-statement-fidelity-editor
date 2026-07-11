from PIL import ImageGrab
import sys

def take_screenshot():
    try:
        screenshot = ImageGrab.grab(all_screens=True)
        screenshot.save("C:\\Users\\home\\.gemini\\antigravity-ide\\brain\\ea44de55-6929-48a9-9279-f3025e2a032d\\desktop_screenshot.png")
        print("Screenshot saved.")
    except Exception as e:
        print(f"Failed to take screenshot: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    take_screenshot()
