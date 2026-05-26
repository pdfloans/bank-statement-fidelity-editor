use rsa::pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use rand::thread_rng;
use std::fs;

#[test]
fn generate_the_key() {
    let mut rng = thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("failed to generate a key");
    let pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
    
    let json = serde_json::json!({
        "type": "service_account",
        "project_id": "test-project",
        "private_key_id": "fake_key_id",
        "private_key": pem,
        "client_email": "fake-service-account@test-project.iam.gserviceaccount.com",
        "client_id": "123456789012345678901",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token",
        "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
        "client_x509_cert_url": "https://www.googleapis.com/robot/v1/metadata/x509/fake-service-account%40test-project.iam.gserviceaccount.com"
    });
    
    let _ = fs::create_dir_all("tests/fixtures");
    fs::write("tests/fixtures/test_service_account.json", serde_json::to_string_pretty(&json).unwrap()).unwrap();
}
