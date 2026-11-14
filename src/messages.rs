pub struct Table {
    pub labels: &'static [(&'static str, &'static str)],
    pub texts: &'static [(&'static str, &'static str)],
}

pub const EN: Table = Table {
    labels: &[
        ("person", "Person"),
        ("cat", "Cat"),
        ("dog", "Dog"),
        ("bird", "Bird"),
        ("car", "Car"),
        ("bicycle", "Bicycle"),
    ],
    texts: &[
        ("motion_title", "Motion detected"),
        ("object_title", "{labels} detected"),
        ("at_time", "Detected at {time}"),
        ("test_title", "Notification test"),
        ("test_body", "Alerts from {site} are working"),
        ("offline_title", "Camera offline"),
        (
            "offline_body",
            "No images for 5 minutes. If you unplugged it, turn it off in Settings, Camera.",
        ),
        ("back_title", "Camera back"),
        ("back_body", "Images are arriving again"),
    ],
};

pub const ES: Table = Table {
    labels: &[
        ("person", "Persona"),
        ("cat", "Gato"),
        ("dog", "Perro"),
        ("bird", "P\u{e1}jaro"),
        ("car", "Coche"),
        ("bicycle", "Bicicleta"),
    ],
    texts: &[
        ("motion_title", "Movimiento detectado"),
        ("object_title", "{labels} detectado"),
        ("at_time", "Detectado a las {time}"),
        ("test_title", "Prueba de avisos"),
        ("test_body", "Los avisos de {site} funcionan"),
        ("offline_title", "C\u{e1}mara sin conexi\u{f3}n"),
        (
            "offline_body",
            "No llegan im\u{e1}genes desde hace 5 minutos. Si la has desenchufado, desact\u{ed}vala en Ajustes, C\u{e1}mara.",
        ),
        ("back_title", "C\u{e1}mara de vuelta"),
        ("back_body", "Vuelven a llegar im\u{e1}genes"),
    ],
};

pub fn table(language: &str) -> &'static Table {
    match language {
        "es" => &ES,
        _ => &EN,
    }
}

pub fn translate(language: &str, key: &str, values: &[(&str, &str)]) -> String {
    let template = table(language)
        .texts
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or(key);
    values
        .iter()
        .fold(template.to_string(), |text, (name, value)| {
            text.replace(&format!("{{{name}}}"), value)
        })
}

fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

pub fn label_names(language: &str, labels: &[String]) -> String {
    let names = table(language).labels;
    labels
        .iter()
        .map(|label| {
            names
                .iter()
                .find(|(k, _)| *k == label)
                .map(|(_, v)| v.to_string())
                .unwrap_or_else(|| capitalize(label))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_are_localised_and_labelled() {
        let labels = vec!["person".to_string(), "cat".to_string()];
        assert_eq!(
            translate(
                "es",
                "object_title",
                &[("labels", &label_names("es", &labels))]
            ),
            "Persona, Gato detectado"
        );
        assert_eq!(
            translate(
                "en",
                "object_title",
                &[("labels", &label_names("en", &["dog".to_string()]))]
            ),
            "Dog detected"
        );
        assert_eq!(
            label_names("en", &["traffic light".to_string()]),
            "Traffic light"
        );
        assert_eq!(label_names("es", &["teddy bear".to_string()]), "Teddy bear");
    }

    #[test]
    fn unknown_languages_fall_back_to_english() {
        assert_eq!(translate("fr", "test_title", &[]), "Notification test");
        assert_eq!(translate("es", "test_title", &[]), "Prueba de avisos");
        assert_eq!(
            translate("en", "test_body", &[("site", "Home")]),
            "Alerts from Home are working"
        );
        assert_eq!(
            translate("es", "at_time", &[("time", "07:05")]),
            "Detectado a las 07:05"
        );
        assert_eq!(
            translate("es", "offline_title", &[]),
            "C\u{e1}mara sin conexi\u{f3}n"
        );
    }
}
