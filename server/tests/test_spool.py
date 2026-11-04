from camd.spool import Spool


def test_frames_roundtrip_in_order(tmp_path):
    spool = Spool.create(tmp_path / "x.spool")
    for i in range(5):
        spool.append(100.0 + i, bytes([i]) * (10 + i))
    spool.close()
    assert spool.count == 5 and spool.first_ts == 100.0 and spool.last_ts == 104.0
    assert [(ts, len(jpeg)) for ts, jpeg in spool.frames()] == [(100.0 + i, 10 + i) for i in range(5)]


def test_a_truncated_spool_loads_what_is_intact(tmp_path):
    spool = Spool.create(tmp_path / "x.spool")
    for i in range(3):
        spool.append(float(i + 1), b"a" * 50)
    spool.close()
    data = spool.path.read_bytes()
    spool.path.write_bytes(data[:-20])
    loaded = Spool.load(spool.path)
    assert loaded.count == 2 and loaded.last_ts == 2.0


def test_delete_removes_the_file(tmp_path):
    spool = Spool.create(tmp_path / "x.spool")
    spool.append(1.0, b"z")
    spool.delete()
    assert not spool.path.exists()
