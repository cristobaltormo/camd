from frames import jpeg, scene

from camd.motion import WARMUP_FRAMES, MotionDetector, MotionSettings


def warmed(detector: MotionDetector, brightness: int = 90) -> None:
    for _ in range(WARMUP_FRAMES + 5):
        assert not detector.process(jpeg(scene(brightness=brightness))).motion


def test_static_scene_has_no_motion():
    d = MotionDetector()
    warmed(d)
    for _ in range(30):
        assert not d.process(jpeg(scene())).motion


def test_person_sized_blob_triggers():
    d = MotionDetector()
    warmed(d)
    assert d.process(jpeg(scene(blob=True))).motion


def test_small_change_ignored_at_low_sensitivity():
    d = MotionDetector(MotionSettings(sensitivity=1))
    warmed(d)
    img = scene()
    img[300:320, 600:620] = 10
    assert not d.process(jpeg(img)).motion


def test_global_light_change_is_not_motion():
    d = MotionDetector()
    warmed(d)
    assert not d.process(jpeg(scene(brightness=200))).motion
    for _ in range(5):
        assert not d.process(jpeg(scene(brightness=200))).motion


def test_ignore_rect_masks_motion():
    d = MotionDetector(MotionSettings(ignore_rects=[[0.3, 0.3, 0.4, 0.6]]))
    warmed(d)
    assert not d.process(jpeg(scene(blob=True))).motion


def test_background_adapts_to_permanent_change():
    d = MotionDetector()
    warmed(d)
    moved = jpeg(scene(blob=True))
    results = [d.process(moved).motion for _ in range(200)]
    assert results[0] and not results[-1]


def test_slow_exposure_drift_is_not_motion():
    d = MotionDetector()
    warmed(d, brightness=90)
    for step in range(1, 12):
        assert not d.process(jpeg(scene(brightness=90 + step * 2))).motion


def test_partial_light_still_counts_as_change_when_it_is_large():
    d = MotionDetector()
    warmed(d)
    img = scene()
    img[150:450, 300:700] = 230
    assert d.process(jpeg(img)).motion


def test_a_change_of_resolution_restarts_the_background_instead_of_failing():
    import cv2

    d = MotionDetector()
    warmed(d)
    small = cv2.resize(scene(), (640, 480))
    assert not d.process(jpeg(small)).motion
    for _ in range(3):
        assert not d.process(jpeg(small)).motion
