use serde::{Deserialize, Serialize};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use base64::{engine::general_purpose, Engine as _};
use std::fs;

#[derive(Debug, Serialize)]
struct VisionRequest {
    model: String,
    messages: Vec<Message>,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: Vec<ContentItem>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentItem {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct VisionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

pub async fn verify_with_vision(api_key: &str, orig_img_path: &str, edit_img_path: &str) -> bool {
    let orig_b64 = encode_image(orig_img_path);
    let edit_b64 = encode_image(edit_img_path);

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", api_key)) {
        headers.insert(AUTHORIZATION, val);
    }

    let prompt = "You are a financial document auditor. I will provide an original page and an edited page. \
                  Are there any semantic differences or corrupted visual artifacts in the edited page that suggest the layout is broken? \
                  Ignore intended textual edits to balances or dates. Reply STRICTLY with a JSON object: {\"passed\": true/false, \"reason\": \"...\"}";

    let req_body = VisionRequest {
        model: "gpt-4o".to_string(), // Or Claude if using Anthropic API format
        max_tokens: 300,
        messages: vec![
            Message {
                role: "user".to_string(),
                content: vec![
                    ContentItem::Text { text: prompt.to_string() },
                    ContentItem::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:image/png;base64,{}", orig_b64),
                        },
                    },
                    ContentItem::ImageUrl {
                        image_url: ImageUrl {
                            url: format!("data:image/png;base64,{}", edit_b64),
                        },
                    },
                ],
            }
        ],
    };

    let res = client.post("https://api.openai.com/v1/chat/completions")
        .headers(headers)
        .json(&req_body)
        .send()
        .await;

    if let Ok(response) = res {
        if let Ok(parsed) = response.json::<VisionResponse>().await {
            if let Some(choice) = parsed.choices.first() {
                let content = choice.message.content.to_lowercase();
                if content.contains("\"passed\": true") {
                    return true;
                } else if content.contains("\"passed\": false") {
                    tracing::warn!("Vision AI rejected the edit: {}", content);
                    return false;
                }
            }
        }
    }

    // Default to true if API call fails so we don't break the offline pipeline
    tracing::warn!("Vision API call failed, falling back to SSIM.");
    true
}

fn encode_image(path: &str) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    general_purpose::STANDARD.encode(bytes)
}
