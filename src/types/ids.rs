/// Generates a newtype wrapping `i64` for use as a typed database ID.
/// Derives Debug, Clone, Copy, PartialEq, Eq, Hash, and implements
/// From<i64> and Into<i64> for database interop.
macro_rules! newtype_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(i64);

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                Self(v)
            }
        }

        impl From<$name> for i64 {
            fn from(v: $name) -> i64 {
                v.0
            }
        }
    };
}

newtype_id!(AccountId);
newtype_id!(JournalEntryId);
newtype_id!(JournalEntryLineId);
newtype_id!(FiscalYearId);
newtype_id!(FiscalPeriodId);
newtype_id!(ArItemId);
newtype_id!(ApItemId);
newtype_id!(EnvelopeAllocationId);
newtype_id!(EnvelopeLedgerId);
newtype_id!(FixedAssetDetailId);
newtype_id!(RecurringTemplateId);
newtype_id!(AuditLogId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_from_into() {
        let id = AccountId::from(42_i64);
        let raw: i64 = id.into();
        assert_eq!(raw, 42);
    }

    #[test]
    fn equality() {
        assert_eq!(AccountId::from(1), AccountId::from(1));
        assert_ne!(AccountId::from(1), AccountId::from(2));
    }

    #[test]
    fn hash_usable_in_hashmap() {
        use std::collections::HashMap;
        let mut map: HashMap<AccountId, &str> = HashMap::new();
        map.insert(AccountId::from(1), "Cash");
        assert_eq!(map[&AccountId::from(1)], "Cash");
    }

    // Compile-time test: this function accepts AccountId, not JournalEntryId.
    // Calling it with JournalEntryId::from(1) would not compile.
    fn _accepts_account_id(_: AccountId) {}

    #[test]
    fn type_safety_compile_check() {
        // This compiles fine — correct type.
        _accepts_account_id(AccountId::from(99));
        // The following would NOT compile (type mismatch):
        // _accepts_account_id(JournalEntryId::from(99));
    }
}
