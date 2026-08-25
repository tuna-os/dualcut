"""Unit tests for .github/scripts/update-index.py — the Flatpak OCI index updater.

The script runs twice in publish-flatpak.yml (once per architecture) against
the tuna-os/docs central flatpak index. It had zero test coverage — a
regression would silently break the publishing pipeline (the same gap class as
tuna-os/letters#13, fixed there with 21 tests).

Tests run the script as a subprocess against a fixture OCI layout and assert
the resulting index/static file byte-for-byte behaviour: creation, append,
per-architecture replacement (the two publish invocations are exactly this),
label filtering, error paths, and idempotency.
"""

import json
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = (
    Path(__file__).resolve().parent.parent
    / ".github" / "scripts" / "update-index.py"
)

MANIFEST_DIGEST = "manifesthash1234"
CONFIG_DIGEST = "confighash5678"


def make_oci_layout(tmp_path, arch="amd64", os_="linux", labels=None,
                    manifest_digest=MANIFEST_DIGEST, config_digest=CONFIG_DIGEST):
    """Build a minimal OCI layout (index.json + blobs) in tmp_path/oci."""
    oci = tmp_path / "oci"
    blobs = oci / "blobs" / "sha256"
    blobs.mkdir(parents=True, exist_ok=True)

    (oci / "index.json").write_text(json.dumps({
        "schemaVersion": 2,
        "manifests": [{"digest": f"sha256:{manifest_digest}", "mediaType": "x"}],
    }))

    (blobs / manifest_digest).write_text(json.dumps({
        "schemaVersion": 2,
        "config": {"digest": f"sha256:{config_digest}", "mediaType": "y"},
    }))

    (blobs / config_digest).write_text(json.dumps({
        "architecture": arch,
        "os": os_,
        "config": {"Labels": labels or {}},
    }))
    return oci


def run_script(oci_dir, index_file, repo_name, tags=None, registry=None):
    argv = [
        sys.executable, str(SCRIPT),
        "--oci-dir", str(oci_dir),
        "--index-file", str(index_file),
        "--repo-name", repo_name,
    ]
    if tags:
        argv += ["--tags", *tags]
    if registry:
        argv += ["--registry", registry]
    return subprocess.run(argv, capture_output=True, text=True)


def flatpak_labels(extra=None):
    labels = {
        "org.flatpak.ref": "app/org.example.App/x86_64/stable",
        "org.flatpak.metadata": "Metadata-version: 1.0\n",
        "com.example.custom": "not-exported",
    }
    labels.update(extra or {})
    return labels


# ── error paths ────────────────────────────────────────────────────────────

class TestErrors:
    def test_missing_index_json_exits_nonzero(self, tmp_path):
        res = run_script(tmp_path / "nope", tmp_path / "index.json", "tuna-os/x")
        assert res.returncode != 0
        assert "index.json not found" in res.stderr

    def test_empty_manifests_exits_nonzero(self, tmp_path):
        oci = tmp_path / "oci"
        oci.mkdir()
        (oci / "index.json").write_text(json.dumps({"manifests": []}))
        res = run_script(oci, tmp_path / "index.json", "tuna-os/x")
        assert res.returncode != 0
        assert "No manifests found" in res.stderr

    def test_missing_required_label_exits_nonzero(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels={"org.flatpak.ref": "x"})
        res = run_script(oci, tmp_path / "index.json", "tuna-os/x")
        assert res.returncode != 0
        assert "Missing required label" in res.stderr


# ── index creation ─────────────────────────────────────────────────────────

class TestCreate:
    def test_creates_new_index_with_defaults(self, tmp_path):
        oci = make_oci_layout(tmp_path, arch="amd64", labels=flatpak_labels())
        index = tmp_path / "index.json"
        res = run_script(oci, index, "tuna-os/dualcut")
        assert res.returncode == 0, res.stderr

        data = json.loads(index.read_text())
        assert data["Registry"] == "https://ghcr.io"
        results = data["Results"]
        assert len(results) == 1
        entry = results[0]
        assert entry["Name"] == "tuna-os/dualcut"
        img = entry["Images"][0]
        assert img["Digest"] == f"sha256:{MANIFEST_DIGEST}"
        assert img["OS"] == "linux"
        assert img["Architecture"] == "amd64"
        assert img["Tags"] == ["latest"]

    def test_filters_to_org_flatpak_labels(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        run_script(oci, index, "tuna-os/dualcut")
        img = json.loads(index.read_text())["Results"][0]["Images"][0]
        assert set(img["Labels"].keys()) == {"org.flatpak.ref", "org.flatpak.metadata"}
        assert "com.example.custom" not in img["Labels"]

    def test_uses_arch_and_os_from_config(self, tmp_path):
        oci = make_oci_layout(tmp_path, arch="aarch64", os_="linux", labels=flatpak_labels())
        index = tmp_path / "index.json"
        run_script(oci, index, "tuna-os/dualcut")
        img = json.loads(index.read_text())["Results"][0]["Images"][0]
        assert img["Architecture"] == "aarch64"
        assert img["OS"] == "linux"

    def test_custom_registry_and_tags(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        res = run_script(oci, index, "tuna-os/dualcut", tags=["stable", "edge"],
                         registry="ghcr.io/example")
        assert res.returncode == 0, res.stderr
        data = json.loads(index.read_text())
        assert data["Registry"] == "https://ghcr.io/example"
        assert data["Results"][0]["Images"][0]["Tags"] == ["stable", "edge"]

    def test_custom_registry_with_scheme(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        res = run_script(oci, index, "tuna-os/dualcut", registry="https://ghcr.io/example")
        assert res.returncode == 0, res.stderr
        data = json.loads(index.read_text())
        assert data["Registry"] == "https://ghcr.io/example"


# ── update semantics (the two publish invocations) ─────────────────────────

class TestUpdate:
    def test_second_arch_replaces_same_arch_keeps_other(self, tmp_path):
        # First publish: amd64
        oci_x = make_oci_layout(tmp_path, arch="amd64", labels=flatpak_labels(),
                                manifest_digest="amd64manifest", config_digest="amd64config")
        index = tmp_path / "index.json"
        run_script(oci_x, index, "tuna-os/dualcut", tags=["latest"])

        # Second publish: aarch64 (a different OCI dir, same index file)
        oci_a = make_oci_layout(tmp_path, arch="aarch64", labels=flatpak_labels(),
                                manifest_digest="arm64manifest", config_digest="arm64config")
        run_script(oci_a, index, "tuna-os/dualcut", tags=["latest"])

        imgs = json.loads(index.read_text())["Results"][0]["Images"]
        arches = sorted(i["Architecture"] for i in imgs)
        assert arches == ["aarch64", "amd64"]

        # Re-publishing amd64 replaces only the amd64 entry (same flow as the
        # workflow re-running per arch)
        run_script(oci_x, index, "tuna-os/dualcut", tags=["latest"])
        imgs = json.loads(index.read_text())["Results"][0]["Images"]
        assert len(imgs) == 2
        amd64 = [i for i in imgs if i["Architecture"] == "amd64"]
        assert len(amd64) == 1
        assert amd64[0]["Digest"] == "sha256:amd64manifest"

    def test_appends_new_repo(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        index.write_text(json.dumps({
            "Registry": "https://ghcr.io",
            "Results": [{"Name": "tuna-os/other", "Images": []}],
        }))
        run_script(oci, index, "tuna-os/dualcut")
        names = [r["Name"] for r in json.loads(index.read_text())["Results"]]
        assert names == ["tuna-os/other", "tuna-os/dualcut"]

    def test_existing_other_repo_untouched(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        original = {
            "Registry": "https://ghcr.io",
            "Results": [{
                "Name": "tuna-os/other",
                "Images": [{"Digest": "sha256:old", "Architecture": "amd64",
                            "OS": "linux", "Tags": ["latest"], "Labels": {}}],
            }],
        }
        index.write_text(json.dumps(original))
        run_script(oci, index, "tuna-os/dualcut")
        other = [r for r in json.loads(index.read_text())["Results"]
                 if r["Name"] == "tuna-os/other"][0]
        assert other["Images"][0]["Digest"] == "sha256:old"


# ── idempotency & output shape ─────────────────────────────────────────────

class TestIdempotency:
    def test_repeated_run_produces_identical_bytes(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        run_script(oci, index, "tuna-os/dualcut")
        first = index.read_bytes()
        run_script(oci, index, "tuna-os/dualcut")
        assert index.read_bytes() == first

    def test_output_ends_with_newline_and_is_indented(self, tmp_path):
        oci = make_oci_layout(tmp_path, labels=flatpak_labels())
        index = tmp_path / "index.json"
        run_script(oci, index, "tuna-os/dualcut")
        raw = index.read_text()
        assert raw.endswith("\n")
        assert '  "Registry"' in raw  # indent=2
