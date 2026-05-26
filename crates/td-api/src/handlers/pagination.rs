//! Pagination query params.
//!
//! Page response shapes are defined per-resource (utoipa generics are
//! awkward enough that a 4-field struct per list endpoint is the cheaper
//! option).

use serde::Deserialize;
use utoipa::IntoParams;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default, rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct Pagination {
    /// 1-indexed page number.
    pub page: u32,
    /// Items per page (capped server-side at 200).
    pub page_size: u32,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

impl Pagination {
    pub fn page(&self) -> u32 {
        self.page.max(1)
    }
    pub fn page_size(&self) -> u32 {
        self.page_size.clamp(1, MAX_PAGE_SIZE)
    }
    pub fn offset(&self) -> u64 {
        (self.page().saturating_sub(1) as u64) * self.page_size() as u64
    }
    pub fn limit(&self) -> u64 {
        self.page_size() as u64
    }
}
