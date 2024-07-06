use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(rename = "match")]
    pub r#match: String,
}

impl Rule {
    pub fn test(&self, target: &str) -> bool {
        if self.regex {
            unimplemented!()
        } else {
            if self.case_insensitive {
                target.to_lowercase().contains(&self.r#match.to_lowercase())
            } else {
                target.contains(&self.r#match)
            }
        }
    }
}
