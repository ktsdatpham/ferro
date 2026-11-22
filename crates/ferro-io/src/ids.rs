use ferro_core::model::Uid;
use ferro_core::ops::IdSource;

/// UUID v7 from the system clock and OS randomness.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemIds;

impl IdSource for SystemIds {
    fn next_uid(&mut self) -> Uid {
        Uid(uuid::Uuid::now_v7().as_u128())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_ids_are_unique_v7() {
        let mut s = SystemIds;
        let a = s.next_uid();
        let b = s.next_uid();
        assert_ne!(a, b);
        assert_eq!(a.version(), 7);
    }
}
