#[derive(Debug, Clone, Default)]
pub struct RDFTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

impl RDFTriple {
    pub fn new(subject: &str, predicate: &str, object: &str) -> Self {
        RDFTriple {
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
        }
    }
}
