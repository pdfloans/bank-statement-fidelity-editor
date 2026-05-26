use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub page: usize,
    pub line_on_page: usize,
    pub date: String,
    pub raw_text: String,
    pub debit: Option<f64>,
    pub credit: Option<f64>,
    pub running_balance: Option<f64>,
    pub bbox: Option<[f32; 4]>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Provenance {
    DocumentAI { confidence: f32 },
    Manual,
    Computed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedChange {
    pub page: usize,
    pub old_text: String,
    pub new_text: String,
    pub reason: String,
    pub confidence: f32,
    pub affects_subsequent_balances: bool,
    pub bbox: Option<[f32; 4]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposed_change_roundtrips_with_bbox() {
        let change_with_bbox = ProposedChange {
            page: 1,
            old_text: "100.00".into(),
            new_text: "150.00".into(),
            reason: "Adjust".into(),
            confidence: 0.95,
            affects_subsequent_balances: true,
            bbox: Some([10.0, 20.0, 50.0, 40.0]),
        };
        let json = serde_json::to_string(&change_with_bbox).unwrap();
        let decoded: ProposedChange = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bbox, Some([10.0, 20.0, 50.0, 40.0]));

        let change_no_bbox = ProposedChange {
            page: 1,
            old_text: "100.00".into(),
            new_text: "150.00".into(),
            reason: "Adjust".into(),
            confidence: 0.95,
            affects_subsequent_balances: true,
            bbox: None,
        };
        let json = serde_json::to_string(&change_no_bbox).unwrap();
        let decoded: ProposedChange = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bbox, None);
    }
}