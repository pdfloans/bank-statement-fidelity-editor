import re

with open("src/engine/verification.rs", "r") as f:
    content = f.read()

# Replace all_applitools_passed definition
content = content.replace("let mut all_applitools_passed = true;", "let mut all_vision_passed = true;")

# Replace Applitools bridge execution block
old_applitools_block = """        // Multi-Verificational System: Applitools Eyes API (Node Bridge)
        // If the API key is present and enabled in settings, we invoke the bridge.
        // If the bridge fails to execute, we continue without it (falling back to SSIM).
        let mut applitools_passed = true;
        let use_applitools = std::env::var("USE_APPLITOOLS")
            .map(|v| v == "1")
            .unwrap_or(true);
        if use_applitools {
            if let Ok(applitools_key) = std::env::var("APPLITOOLS_API_KEY") {
                if !applitools_key.is_empty() {
                    let ignore_regions: Vec<_> = exclude_rects
                        .iter()
                        .map(|&(x0, y0, x1, y1)| {
                            serde_json::json!({
                                "left": x0,
                                "top": y0,
                                "width": x1.saturating_sub(x0),
                                "height": y1.saturating_sub(y0)
                            })
                        })
                        .collect();

                    let ignore_json = serde_json::to_string(&ignore_regions).unwrap_or_default();
                    let app_name = "Bank Statement Modifier";
                    let test_name = format!("Visual Diff Page {}", i + 1);

                    let out = std::process::Command::new("node")
                        .arg("src/ai/applitools_bridge.js")
                        .arg(&applitools_key)
                        .arg(app_name)
                        .arg(&test_name)
                        .arg(&orig_png_path)
                        .arg(&edit_png_path)
                        .arg(&ignore_json)
                        .output();

                    if let Ok(out) = out {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            if let Some(json_str) = line.strip_prefix("APPLITOOLS_RESULT:") {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                    if let Some(passed) = v.get("passed").and_then(|p| p.as_bool())
                                    {
                                        tracing::info!(
                                            "[verification] Applitools verification passed: {}",
                                            passed
                                        );
                                        applitools_passed = passed;
                                    }
                                }
                            }
                        }
                    } else {
                        tracing::warn!("[verification] Applitools bridge failed to execute. Falling back to local SSIM only.");
                    }
                }
            }
        }
        all_applitools_passed = all_applitools_passed && applitools_passed;"""

new_vision_block = """        // Multi-Verificational System: Vision AI (Claude 3.5 Sonnet / GPT-4o)
        let mut vision_passed = true;
        let use_vision = std::env::var("USE_VISION_AI").map(|v| v == "1").unwrap_or(true);
        if use_vision {
            if let Ok(vision_key) = std::env::var("VISION_API_KEY") {
                if !vision_key.is_empty() {
                    let passed = crate::ai::vision::verify_with_vision(
                        &vision_key,
                        &orig_png_path.to_string_lossy(),
                        &edit_png_path.to_string_lossy(),
                    ).await;
                    vision_passed = passed;
                }
            }
        }
        all_vision_passed = all_vision_passed && vision_passed;"""

content = content.replace(old_applitools_block, new_vision_block)

# Replace the && all_applitools_passed
content = content.replace("&& all_applitools_passed", "&& all_vision_passed")

# Remove applitools test at the bottom
test_start = content.find("    // Phase 6 - Applitools graceful degradation")
test_end = content.find("}", content.find("fn applitools_bridge_missing_does_not_crash() {")) + 1
test_end2 = content.find("}", test_end) + 1 # Need one more brace for the function block closure

if test_start != -1:
    content = content[:test_start] + content[test_end2:]

with open("src/engine/verification.rs", "w") as f:
    f.write(content)
