pub struct Brand {
    pub name: &'static str,
    pub day: &'static str,
    pub short: &'static str,
    pub intro: &'static str,
    pub date: &'static str,
    pub reset: &'static str,
    pub month: i64,
    pub day_of_month: i64,
    pub info: &'static str,
}

pub const BRANDS: [Brand; 2] = [
    Brand {
        name: "petrov",
        day: "Petrov Day",
        short: "Petrov",
        intro: "On 26 September 1983, Stanislav Petrov decided not to pass a missile warning up the chain of command.",
        date: "26 September",
        reset: "27 September, 12:00 UTC",
        month: 9,
        day_of_month: 26,
        info: include_str!("../static/brands/petrov/info.html"),
    },
    Brand {
        name: "arkhipov",
        day: "Arkhipov Day",
        short: "Arkhipov",
        intro: "On 27 October 1962, Vasili Arkhipov refused to agree to launching a nuclear torpedo from the Soviet submarine B-59.",
        date: "27 October",
        reset: "28 October, 12:00 UTC",
        month: 10,
        day_of_month: 27,
        info: include_str!("../static/brands/arkhipov/info.html"),
    },
];

impl Brand {
    pub fn find(name: &str) -> Option<&'static Brand> {
        BRANDS.iter().find(|b| b.name == name)
    }

    pub fn render(&self, template: &str) -> String {
        template
            .replace("{{INFO}}", self.info)
            .replace("{{INTRO}}", self.intro)
            .replace("{{DAY}}", self.day)
            .replace("{{SHORT}}", self.short)
            .replace("{{DATE}}", self.date)
            .replace("{{RESET}}", self.reset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = include_str!("../static/index.html");
    const MANIFEST: &str = include_str!("../static/manifest.webmanifest");

    #[test]
    fn every_placeholder_is_filled_for_every_brand() {
        for brand in &BRANDS {
            for template in [INDEX, MANIFEST] {
                let page = brand.render(template);
                assert!(!page.contains("{{"), "{} left a placeholder", brand.name);
            }
        }
    }

    #[test]
    fn arkhipov_pages_never_mention_petrov() {
        let arkhipov = Brand::find("arkhipov").unwrap();
        for template in [INDEX, MANIFEST] {
            let page = arkhipov.render(template);
            assert!(!page.contains("Petrov") && !page.contains("September"));
        }
        assert!(
            Brand::find("petrov")
                .unwrap()
                .render(INDEX)
                .contains("Petrov Day")
        );
    }
}
