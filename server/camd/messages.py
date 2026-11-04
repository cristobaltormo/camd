TEXT = {
    "en": {
        "labels": {"person": "Person", "cat": "Cat", "dog": "Dog", "bird": "Bird", "car": "Car", "bicycle": "Bicycle"},
        "motion_title": "Motion detected",
        "object_title": "{labels} detected",
        "at_time": "Detected at {time}",
        "test_title": "Notification test",
        "test_body": "Alerts from {site} are working",
        "offline_title": "Camera offline",
        "offline_body": "No images for 5 minutes. If you unplugged it, turn it off in Settings, Camera.",
        "back_title": "Camera back",
        "back_body": "Images are arriving again",
    },
    "es": {
        "labels": {"person": "Persona", "cat": "Gato", "dog": "Perro", "bird": "Pájaro", "car": "Coche", "bicycle": "Bicicleta"},
        "motion_title": "Movimiento detectado",
        "object_title": "{labels} detectado",
        "at_time": "Detectado a las {time}",
        "test_title": "Prueba de avisos",
        "test_body": "Los avisos de {site} funcionan",
        "offline_title": "Cámara sin conexión",
        "offline_body": "No llegan imágenes desde hace 5 minutos. Si la has desenchufado, desactívala en Ajustes, Cámara.",
        "back_title": "Cámara de vuelta",
        "back_body": "Vuelven a llegar imágenes",
    },
}


def translate(language: str, key: str, **values: str) -> str:
    table = TEXT.get(language, TEXT["en"])
    return table[key].format(**values)


def label_names(language: str, labels: list[str]) -> str:
    names = TEXT.get(language, TEXT["en"])["labels"]
    return ", ".join(names.get(label, label.capitalize()) for label in labels)
