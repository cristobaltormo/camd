import re
import tomllib
from pathlib import Path

from camd import __version__

ROOT = Path(__file__).resolve().parent.parent.parent


def test_package_and_module_versions_agree():
    project = tomllib.loads((ROOT / "server" / "pyproject.toml").read_text())["project"]
    assert project["version"] == __version__


def test_changelog_documents_the_current_version():
    headings = re.findall(r"^## (\S+)", (ROOT / "CHANGELOG.md").read_text(), re.M)
    assert __version__ in headings
