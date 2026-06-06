use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY must be set");
    
    let url_with_query = format!("https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent?key={}", api_key);
    let url_without_query = "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent";

    let body = serde_json::json!({
        "contents": [{ "parts": [{ "text": "Hello, world!" }] }]
    });

    println!("Testing with query param...");
    let client = reqwest::Client::new();
    let resp1 = client.post(&url_with_query).json(&body).send().await?;
    println!("Status query param: {}", resp1.status());
    println!("Response: {}\n", resp1.text().await?);

    println!("Testing with header...");
    let mut headers = HeaderMap::new();
    headers.insert("x-goog-api-key", HeaderValue::from_str(&api_key)?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    
    let resp2 = client.post(url_without_query).headers(headers).json(&body).send().await?;
    println!("Status header: {}", resp2.status());
    println!("Response: {}\n", resp2.text().await?);

    Ok(())
}
