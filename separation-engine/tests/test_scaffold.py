from stemsplitter_engine import PROTOCOL_VERSION


def test_protocol_version_pinned():
    assert PROTOCOL_VERSION == 1
