use std::cmp::Ordering;

pub const EPS: f32 = 1e-6;

pub trait RelativeCompare {
    fn relative_cmp(&self, rhs: &f32) -> Ordering;
}

impl RelativeCompare for f32 {
    fn relative_cmp(&self, rhs: &f32) -> Ordering {
        let diff = self - rhs;
        if diff.abs() < EPS {
            Ordering::Equal
        }
        else if diff < 0.0 {
            Ordering::Less
        }
        else {
            Ordering::Greater
        }
    }
}