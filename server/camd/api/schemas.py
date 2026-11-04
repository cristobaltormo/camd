from pydantic import BaseModel, Field, StrictBool

MAX_CLIP_S = 300


class CodeBody(BaseModel):
    code: str = Field(min_length=4, max_length=64)


class RegisterBody(CodeBody):
    credential: dict
    name: str = Field(default="", max_length=40)


class LoginBody(BaseModel):
    cid: str
    credential: dict


class PasswordBody(BaseModel):
    password: str = Field(min_length=1, max_length=256)


class ModeBody(BaseModel):
    mode: str


class SnoozeBody(BaseModel):
    minutes: float = Field(ge=0, le=1440)


class PrivacyBody(BaseModel):
    on: bool


class SettingsBody(BaseModel):
    model_config = {"extra": "forbid"}

    sensitivity: int | None = Field(default=None, ge=1, le=10)
    ignore_rects: list[list[float]] | None = Field(default=None, max_length=10)
    retention_days: int | None = Field(default=None, ge=1, le=365)
    pre_roll_s: float | None = None
    post_roll_s: float | None = None
    max_event_s: float | None = None
    max_disk_gb: float | None = None
    presence_grace_s: float | None = None
    push_cooldown_s: float | None = None
    unconfirmed_keep_h: float | None = None
    detect_confidence: float | None = None
    detection: str | None = None
    detect_labels: list[str] | None = None
    language: str | None = None
    notify_motion: StrictBool | None = None
    notify_offline: StrictBool | None = None
    quiet_enabled: StrictBool | None = None
    quiet_start: str | None = None
    quiet_end: str | None = None
    camera_enabled: StrictBool | None = None
    camera_vflip: StrictBool | None = None
    camera_hmirror: StrictBool | None = None
    camera_brightness: int | None = None
    camera_contrast: int | None = None
    camera_quality: int | None = None


class IdsBody(BaseModel):
    ids: list[int] = Field(min_length=1, max_length=200)


class ClipBody(BaseModel):
    seconds: int = Field(default=30, ge=5, le=MAX_CLIP_S)


class KeepBody(BaseModel):
    keep: bool


class PushBody(BaseModel):
    subscription: dict


class EndpointBody(BaseModel):
    endpoint: str
