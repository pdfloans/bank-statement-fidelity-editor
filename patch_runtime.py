import sys

def main():
    with open("src/app/runtime.rs", "r") as f:
        code = f.read()

    # 1. Add InteractiveFallbackResponse to Job
    code = code.replace(
        "    AiConfirmationResponse(crate::engine::ai_confirm::AiConfirmationResponse),",
        "    AiConfirmationResponse(crate::engine::ai_confirm::AiConfirmationResponse),\n    InteractiveFallbackResponse(crate::engine::interactive_fallback::InteractiveFallbackResponse),"
    )

    # 2. Add InteractiveFallbackRequired to JobResult
    code = code.replace(
        "    AiConfirmationNeeded(crate::engine::ai_confirm::AiConfirmation),",
        "    AiConfirmationNeeded(crate::engine::ai_confirm::AiConfirmation),\n    InteractiveFallbackRequired(crate::engine::interactive_fallback::InteractiveFallbackRequest),"
    )

    # 3. Add fallback_router
    code = code.replace(
        "            let mut segment_manager: Option<SegmentManager> = None;\n\n            while let Some(job) = tokio_job_rx.recv().await {",
        "            let mut segment_manager: Option<SegmentManager> = None;\n            let fallback_router: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<uuid::Uuid, tokio::sync::oneshot::Sender<String>>>> = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));\n\n            while let Some(job) = tokio_job_rx.recv().await {"
    )

    # 4. Handle InteractiveFallbackResponse
    handler = """                        let _ = crate::engine::ai_confirm::log_learning_response(&placeholder_confirmation, &response);
                    }
                    Job::InteractiveFallbackResponse(response) => {
                        let id = response.id;
                        let router = fallback_router.clone();
                        tokio::spawn(async move {
                            let mut map = router.lock().await;
                            if let Some(tx) = map.remove(&id) {
                                let _ = tx.send(response.selected_alternative_id);
                            }
                        });
                    }"""
    code = code.replace(
        "                        let _ = crate::engine::ai_confirm::log_learning_response(&placeholder_confirmation, &response);\n                    }",
        handler
    )
    
    # 5. Pass router into WorkflowParseAndValidate
    code = code.replace(
        "                        let engine_for_tokio = engine_for_tokio.clone();\n                        tokio::spawn(async move {",
        "                        let engine_for_tokio = engine_for_tokio.clone();\n                        let router = fallback_router.clone();\n                        tokio::spawn(async move {"
    )

    with open("src/app/runtime.rs", "w") as f:
        f.write(code)
    print("Patched.")

if __name__ == "__main__":
    main()
