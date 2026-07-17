import sys

def main():
    with open("src/app/config.rs", "r") as f:
        code = f.read()

    # Find where AppConfig is defined
    target1 = "    pub interactive_fallbacks: bool,"
    replacement1 = """    pub interactive_fallbacks: bool,
    pub transfer_consensus_mode: bool,
    pub auto_match_dpi: bool,"""
    code = code.replace(target1, replacement1)

    target2 = "            interactive_fallbacks: true,"
    replacement2 = """            interactive_fallbacks: true,
            transfer_consensus_mode: true,
            auto_match_dpi: false,"""
    code = code.replace(target2, replacement2)

    with open("src/app/config.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()
