use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeCard {
    pub name: String,
    pub ingredients: Vec<String>,
    pub instructions: Vec<String>,
    pub prep_time: Option<String>,
    pub cook_time: Option<String>,
    pub servings: Option<String>,
    pub image_url: Option<String>,
}
