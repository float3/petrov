use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier},
};
use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::game::random_key;

pub const MAX_GROUPS: usize = 1000;

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

fn check(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let len = value.trim().chars().count();
    if len < min {
        return Err(format!("Please fill in the {field}."));
    }
    if len > max {
        return Err(format!("The {field} can have at most {max} characters."));
    }
    Ok(())
}

impl GroupInput {
    pub fn validate(&self) -> Result<(), String> {
        check("name", &self.name, 1, 80)?;
        check("description", &self.description, 1, 2000)?;
        check("time", &self.when, 1, 120)?;
        check("location or time zone", &self.location, 1, 120)?;
        check("contact", &self.contact, 1, 200)?;
        if self.password.chars().count() < 6 {
            return Err("The password needs at least 6 characters.".into());
        }
        if self.password.len() > 200 {
            return Err("The password can have at most 200 characters.".into());
        }
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
    fn validation_rejects_short_passwords_and_empty_fields() {
        assert!(input("hunter42").validate().is_ok());
        assert!(input("short").validate().is_err());
        let mut empty = input("hunter42");
        empty.contact = "  ".into();
        assert!(empty.validate().is_err());
    }
}
