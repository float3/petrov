use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::game::random_key;

pub const MAX_GROUPS: usize = 1000;

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn reset_of_year(year: i64, month: i64, day: i64) -> u64 {
    (((days_from_civil(year, month, day) + 1) * 86_400 + 12 * 3600) * 1000) as u64
}

/// The most recent moment the given day ended everywhere on Earth: midnight
/// at UTC-12, which is 12:00 UTC the next day. Listings from before it are cleared.
pub fn last_reset(now: u64, month: i64, day: i64) -> u64 {
    let mut year = 1970 + (now / 31_556_952_000) as i64;
    while reset_of_year(year, month, day) > now {
        year -= 1;
    }
    while reset_of_year(year + 1, month, day) <= now {
        year += 1;
    }
    reset_of_year(year, month, day)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: String,
    pub when: String,
    pub location: String,
    pub contact: String,
    pub password_hash: String,
    pub created: u64,
    pub updated: u64,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct PublicGroup {
    pub id: String,
    pub name: String,
    pub description: String,
    pub when: String,
    pub location: String,
    pub contact: String,
    pub created: u64,
    pub updated: u64,
}

#[derive(Deserialize)]
pub struct GroupInput {
    pub name: String,
    pub description: String,
    pub when: String,
    pub location: String,
    pub contact: String,
    pub password: String,
}

fn check(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.chars().count() > max {
        return Err(format!("The {field} can have at most {max} characters."));
    }
    Ok(())
}

impl GroupInput {
    pub fn validate(&self) -> Result<(), String> {
        check("name", &self.name, 1000)?;
        check("description", &self.description, 20_000)?;
        check("time", &self.when, 1000)?;
        check("location or time zone", &self.location, 1000)?;
        check("contact", &self.contact, 1000)?;
        check("password", &self.password, 10_000)?;
        Ok(())
    }
}

pub fn hash_password(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2 hashing with default parameters")
        .to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), hash)
        .is_ok()
}

impl Group {
    pub fn new(input: &GroupInput, password_hash: String, now: u64, rng: &mut impl Rng) -> Group {
        Group {
            id: random_key(rng, 16),
            name: input.name.trim().into(),
            description: input.description.trim().into(),
            when: input.when.trim().into(),
            location: input.location.trim().into(),
            contact: input.contact.trim().into(),
            password_hash,
            created: now,
            updated: now,
        }
    }

    pub fn update(&mut self, input: &GroupInput, now: u64) {
        self.name = input.name.trim().into();
        self.description = input.description.trim().into();
        self.when = input.when.trim().into();
        self.location = input.location.trim().into();
        self.contact = input.contact.trim().into();
        self.updated = now;
    }

    pub fn public(&self) -> PublicGroup {
        PublicGroup {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            when: self.when.clone(),
            location: self.location.clone(),
            contact: self.contact.clone(),
            created: self.created,
            updated: self.updated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(password: &str) -> GroupInput {
        GroupInput {
            name: " Munich ".into(),
            description: "We bake a cake.".into(),
            when: "26 Sep, 19:30".into(),
            location: "Munich, CEST".into(),
            contact: "someone@example.com".into(),
            password: password.into(),
        }
    }

    #[test]
    fn listings_reset_once_26_september_is_over_everywhere() {
        let reset_2025 = 1_758_974_400_000;
        let reset_2026 = 1_790_510_400_000;
        assert_eq!(last_reset(1_789_603_200_000, 9, 26), reset_2025);
        assert_eq!(last_reset(reset_2026 - 1, 9, 26), reset_2025);
        assert_eq!(last_reset(reset_2026, 9, 26), reset_2026);
        assert_eq!(last_reset(1_709_164_800_000, 9, 26), 1_695_816_000_000);
        assert_eq!(last_reset(1_789_603_200_000, 10, 27), 1_761_652_800_000);
    }

    #[test]
    fn passwords_verify_and_are_not_stored_in_plain_text() {
        let hash = hash_password("hunter42");
        assert!(!hash.contains("hunter42"));
        assert!(verify_password("hunter42", &hash));
        assert!(!verify_password("hunter43", &hash));
    }

    #[test]
    fn public_view_has_no_password_hash() {
        let mut rng = rand::rng();
        let group = Group::new(&input("hunter42"), hash_password("hunter42"), 5, &mut rng);
        let json = serde_json::to_string(&group.public()).unwrap();
        assert!(!json.contains("argon2") && !json.contains("password"));
        assert_eq!(group.name, "Munich");
    }

    #[test]
    fn validation_accepts_any_password_and_empty_fields() {
        assert!(input("hunter42").validate().is_ok());
        assert!(input("").validate().is_ok());
        assert!(input("a").validate().is_ok());
        let mut empty = input("");
        empty.contact = "  ".into();
        empty.name = String::new();
        assert!(empty.validate().is_ok());
        let mut huge = input("");
        huge.name = "x".repeat(1001);
        assert!(huge.validate().is_err());
    }
}
