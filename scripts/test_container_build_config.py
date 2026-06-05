from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]


class ContainerBuildConfigTests(unittest.TestCase):
    def test_release_container_build_uses_low_memory_release_profile_overrides(
        self,
    ) -> None:
        dockerfile = (REPO_ROOT / "Dockerfile").read_text()

        self.assertIn("ARG CARGO_PROFILE_RELEASE_LTO=thin", dockerfile)
        self.assertIn("ARG CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16", dockerfile)
        self.assertIn(
            'CARGO_PROFILE_RELEASE_LTO="$CARGO_PROFILE_RELEASE_LTO"', dockerfile
        )
        self.assertIn(
            'CARGO_PROFILE_RELEASE_CODEGEN_UNITS="$CARGO_PROFILE_RELEASE_CODEGEN_UNITS"',
            dockerfile,
        )
        self.assertIn(
            "cargo build --locked --release -p codex-cli --bin codex", dockerfile
        )

    def test_compose_passes_release_profile_overrides_to_docker_build(self) -> None:
        compose = (REPO_ROOT / "compose.yaml").read_text()

        self.assertIn(
            "CARGO_PROFILE_RELEASE_LTO: ${CODEX_CARGO_PROFILE_RELEASE_LTO:-thin}",
            compose,
        )
        self.assertIn(
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS: "
            "${CODEX_CARGO_PROFILE_RELEASE_CODEGEN_UNITS:-16}",
            compose,
        )

    def test_runtime_image_installs_system_bubblewrap(self) -> None:
        dockerfile = (REPO_ROOT / "Dockerfile").read_text()

        self.assertIn("bubblewrap", dockerfile)

    def test_container_entrypoint_defaults_to_outer_container_sandbox(self) -> None:
        entrypoint = (REPO_ROOT / "docker-entrypoint.sh").read_text()

        self.assertIn("CODEX_CONTAINER_SANDBOX_MODE", entrypoint)
        self.assertIn("danger-full-access", entrypoint)
        self.assertIn('sandbox_mode=\\"$container_sandbox_mode\\"', entrypoint)


if __name__ == "__main__":
    unittest.main()
