// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cookiecutter-style template definitions bundled with cave-scaffold.
//!
//! This module defines inline, self-contained project templates in the classic
//! Cookiecutter shape (`{{ cookiecutter.var }}` substitution in file paths and
//! file contents). The templates are seeded into the `ScaffoldStore` at
//! startup so the portal and CLI surface them with no external fetch required.
//!
//! The first seeded template is `python-fastapi-service`, which ships with a
//! `pip.conf` and `pyproject.toml` that point at the internal CAVE artifact
//! registry (the Pulp/Harbor replacement, `cave-registry`) instead of the
//! public PyPI. This keeps all Python dependency resolution on the sovereign
//! platform by default.

use crate::models::{
    ParameterType, Template, TemplateCategory, TemplateOutput, TemplateParameter, TemplateStep,
    OutputLink,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Cookiecutter data types
// ---------------------------------------------------------------------------

/// One variable presented to the developer when they invoke the template.
#[derive(Debug, Clone)]
pub struct CookiecutterVariable {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default: Option<&'static str>,
    pub required: bool,
    pub enum_values: &'static [&'static str],
}

/// A single rendered output file in the cookiecutter tree. Both `path` and
/// `content` are rendered through the same `{{ cookiecutter.var }}` substitution
/// pass before being written to the workspace.
#[derive(Debug, Clone)]
pub struct CookiecutterFile {
    pub path: &'static str,
    pub content: &'static str,
}

/// A complete cookiecutter-style template — a file tree plus the variable
/// context it declares.
#[derive(Debug, Clone)]
pub struct CookiecutterTemplate {
    /// Stable slug for referencing the template (e.g. `python-fastapi-service`).
    pub slug: &'static str,
    /// Human-readable title.
    pub title: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// Owning team or identity (e.g. `platform`, `data-platform`).
    pub owner: &'static str,
    /// High-level category the template belongs to.
    pub category: TemplateCategory,
    /// Discovery tags (used for filtering in portal / CLI).
    pub tags: &'static [&'static str],
    /// Variables the user must/can provide when invoking.
    pub context: &'static [CookiecutterVariable],
    /// Files to render. `path` supports variable substitution so directories
    /// like `src/{{ cookiecutter.package_name }}` work as expected.
    pub files: &'static [CookiecutterFile],
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a cookiecutter template with the given variable values into a list
/// of `(path, content)` tuples. Unknown variables are left as-is so rendering
/// never fails; validation is the caller's responsibility.
pub fn render(
    template: &CookiecutterTemplate,
    values: &HashMap<String, String>,
) -> Vec<(String, String)> {
    template
        .files
        .iter()
        .map(|f| {
            let path = substitute(f.path, values);
            let content = substitute(f.content, values);
            (path, content)
        })
        .collect()
}

/// Replace `{{ cookiecutter.key }}` tokens (with or without surrounding
/// whitespace) with values from the map. Only the `cookiecutter.` namespace is
/// honoured, matching Cookiecutter semantics.
pub fn substitute(input: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Look for the closing `}}`.
            if let Some(end) = find_double_brace_end(&bytes[i + 2..]) {
                let raw = &input[i + 2..i + 2 + end];
                let token = raw.trim();
                if let Some(rest) = token.strip_prefix("cookiecutter.") {
                    let key = rest.trim();
                    if let Some(v) = values.get(key) {
                        out.push_str(v);
                    } else {
                        // Unknown var: keep the original token so it is obvious.
                        out.push_str(&input[i..i + 2 + end + 2]);
                    }
                } else {
                    // Not a cookiecutter namespace — keep original.
                    out.push_str(&input[i..i + 2 + end + 2]);
                }
                i += 2 + end + 2;
                continue;
            }
        }
        out.push(input[i..].chars().next().unwrap());
        i += input[i..].chars().next().unwrap().len_utf8();
    }
    out
}

fn find_double_brace_end(slice: &[u8]) -> Option<usize> {
    let mut j = 0;
    while j + 1 < slice.len() {
        if slice[j] == b'}' && slice[j + 1] == b'}' {
            return Some(j);
        }
        j += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Conversion: CookiecutterTemplate → scaffold `Template`
// ---------------------------------------------------------------------------

/// Convert a bundled cookiecutter template into the public `Template` record
/// that the scaffold store tracks and the portal displays. The rendered file
/// tree is reachable through the `cookiecutter:render` step inside the
/// template's steps (handled by the engine at scaffold time).
pub fn to_template(ct: &CookiecutterTemplate) -> Template {
    let now = Utc::now();
    let parameters: Vec<TemplateParameter> = ct
        .context
        .iter()
        .map(|v| TemplateParameter {
            id: v.key.to_string(),
            title: v.label.to_string(),
            description: v.description.to_string(),
            param_type: if v.enum_values.is_empty() {
                ParameterType::String_
            } else {
                ParameterType::Select
            },
            required: v.required,
            default: v
                .default
                .map(|d| serde_json::Value::String(d.to_string())),
            enum_values: v.enum_values.iter().map(|s| s.to_string()).collect(),
            pattern: None,
        })
        .collect();

    // Step 1 — render cookiecutter files into the workspace.
    let mut render_step = TemplateStep::new(
        "render",
        "Render Cookiecutter template",
        "cookiecutter:render",
    );
    render_step
        .input
        .insert("slug".to_string(), serde_json::Value::String(ct.slug.to_string()));

    // Step 2 — publish a new GitHub repo (defaults; can be swapped to gitlab).
    let mut publish_step = TemplateStep::new(
        "publish",
        "Publish to Git repository",
        "publish:github",
    );
    publish_step.input.insert(
        "repoUrl".to_string(),
        serde_json::Value::String(
            "github.com?owner={{ cookiecutter.owner }}&repo={{ cookiecutter.service_name }}".into(),
        ),
    );
    publish_step.input.insert(
        "defaultBranch".to_string(),
        serde_json::Value::String("main".to_string()),
    );

    // Step 3 — create pipeline in cave-pipelines (ci:create-pipeline).
    let mut pipeline_step = TemplateStep::new(
        "pipeline",
        "Provision CI/CD pipeline",
        "ci:create-pipeline",
    );
    pipeline_step.input.insert(
        "repoUrl".to_string(),
        serde_json::Value::String(
            "github.com?owner={{ cookiecutter.owner }}&repo={{ cookiecutter.service_name }}".into(),
        ),
    );
    pipeline_step.input.insert(
        "pipelineType".to_string(),
        serde_json::Value::String("github-actions".to_string()),
    );

    // Step 4 — register in cave-portal component catalog.
    let mut catalog_step = TemplateStep::new(
        "catalog",
        "Register in software catalog",
        "catalog:register",
    );
    catalog_step.input.insert(
        "catalogInfoUrl".to_string(),
        serde_json::Value::String("./catalog-info.yaml".to_string()),
    );

    Template {
        id: Uuid::new_v4(),
        name: ct.slug.to_string(),
        title: ct.title.to_string(),
        description: ct.description.to_string(),
        owner: ct.owner.to_string(),
        tags: ct.tags.iter().map(|s| s.to_string()).collect(),
        category: ct.category.clone(),
        parameters,
        steps: vec![render_step, publish_step, pipeline_step, catalog_step],
        output: TemplateOutput {
            links: vec![
                OutputLink {
                    title: "Repository".to_string(),
                    url: "{{ cookiecutter.repo_url }}".to_string(),
                    icon: Some("github".to_string()),
                },
                OutputLink {
                    title: "Pipeline".to_string(),
                    url: "/portal/pipelines?repo={{ cookiecutter.service_name }}".to_string(),
                    icon: Some("play-circle".to_string()),
                },
                OutputLink {
                    title: "Service in catalog".to_string(),
                    url: "/portal/catalog/{{ cookiecutter.service_name }}".to_string(),
                    icon: Some("cube".to_string()),
                },
            ],
        },
        created_at: now,
        updated_at: now,
        version: "1.0.0".to_string(),
    }
}

/// All built-in cookiecutter templates that ship with cave-scaffold.
pub fn builtin_cookiecutter_templates() -> Vec<CookiecutterTemplate> {
    vec![python_fastapi_service()]
}

// ---------------------------------------------------------------------------
// python-fastapi-service — the first seeded template
// ---------------------------------------------------------------------------

/// `python-fastapi-service` — a production-ready FastAPI microservice scaffold
/// wired for the sovereign CAVE toolchain:
///   * `pip.conf` points at `cave-registry` (internal PyPI mirror, Pulp
///     replacement) as the primary index. Public PyPI is not consulted.
///   * `pyproject.toml` uses the same index for `[tool.uv.index]` and
///     `[tool.hatch.envs]` so `uv sync` and `hatch shell` behave identically.
///   * `Dockerfile` exports `PIP_INDEX_URL` via build args so multi-stage
///     builds inherit the same index.
///   * `cave-pipeline.yaml` invokes the planned python task catalog
///     (`uv-sync`, `pytest`, `ruff`, `mypy`, `bandit`, `buildah`,
///     `trivy-scan`, `sonar-scan`, `publish-image`).
pub fn python_fastapi_service() -> CookiecutterTemplate {
    CookiecutterTemplate {
        slug: "python-fastapi-service",
        title: "Python FastAPI Service",
        description:
            "Production-ready FastAPI microservice scaffold wired for CAVE \
             (sovereign pip index via cave-registry, uv lockfile, pytest, \
             ruff, mypy, bandit, Dockerfile, cave-pipeline.yaml, \
             catalog-info.yaml).",
        owner: "platform",
        category: TemplateCategory::Microservice,
        tags: &[
            "python",
            "fastapi",
            "microservice",
            "cookiecutter",
            "sovereign",
            "cave-registry",
        ],
        context: PYTHON_FASTAPI_CONTEXT,
        files: PYTHON_FASTAPI_FILES,
    }
}

const PYTHON_FASTAPI_CONTEXT: &[CookiecutterVariable] = &[
    CookiecutterVariable {
        key: "service_name",
        label: "Service name",
        description: "Kebab-case service name (used for repo, docker image, k8s).",
        default: Some("my-fastapi-service"),
        required: true,
        enum_values: &[],
    },
    CookiecutterVariable {
        key: "package_name",
        label: "Python package name",
        description: "snake_case Python package name (usually the service name with underscores).",
        default: Some("my_fastapi_service"),
        required: true,
        enum_values: &[],
    },
    CookiecutterVariable {
        key: "description",
        label: "Short description",
        description: "One-line description used in README and pyproject.toml.",
        default: Some("FastAPI microservice on CAVE"),
        required: false,
        enum_values: &[],
    },
    CookiecutterVariable {
        key: "owner",
        label: "Owner / team",
        description: "Team slug that owns the service (used in catalog-info.yaml).",
        default: Some("platform"),
        required: true,
        enum_values: &[],
    },
    CookiecutterVariable {
        key: "python_version",
        label: "Python version",
        description: "Pinned Python version for the base image and uv.",
        default: Some("3.12"),
        required: true,
        enum_values: &["3.11", "3.12", "3.13"],
    },
    CookiecutterVariable {
        key: "cave_registry_host",
        label: "cave-registry hostname",
        description: "Host serving the internal PyPI index (Pulp replacement).",
        default: Some("cave-registry.cave.caveplatform.dev"),
        required: true,
        enum_values: &[],
    },
    CookiecutterVariable {
        key: "image_registry",
        label: "OCI image registry",
        description: "Container registry for pushed images (cave-registry OCI).",
        default: Some("cave-registry.cave.caveplatform.dev"),
        required: true,
        enum_values: &[],
    },
];

const PYTHON_FASTAPI_FILES: &[CookiecutterFile] = &[
    CookiecutterFile {
        path: "pyproject.toml",
        content: PYPROJECT_TOML,
    },
    CookiecutterFile {
        path: "pip.conf",
        content: PIP_CONF,
    },
    CookiecutterFile {
        path: ".pip/pip.conf",
        content: PIP_CONF,
    },
    CookiecutterFile {
        path: "uv.toml",
        content: UV_TOML,
    },
    CookiecutterFile {
        path: "src/{{ cookiecutter.package_name }}/__init__.py",
        content: PACKAGE_INIT,
    },
    CookiecutterFile {
        path: "src/{{ cookiecutter.package_name }}/main.py",
        content: MAIN_PY,
    },
    CookiecutterFile {
        path: "src/{{ cookiecutter.package_name }}/config.py",
        content: CONFIG_PY,
    },
    CookiecutterFile {
        path: "src/{{ cookiecutter.package_name }}/routes/__init__.py",
        content: "",
    },
    CookiecutterFile {
        path: "src/{{ cookiecutter.package_name }}/routes/health.py",
        content: HEALTH_PY,
    },
    CookiecutterFile {
        path: "tests/__init__.py",
        content: "",
    },
    CookiecutterFile {
        path: "tests/test_health.py",
        content: TEST_HEALTH_PY,
    },
    CookiecutterFile {
        path: "Dockerfile",
        content: DOCKERFILE,
    },
    CookiecutterFile {
        path: ".dockerignore",
        content: DOCKERIGNORE,
    },
    CookiecutterFile {
        path: ".gitignore",
        content: GITIGNORE,
    },
    CookiecutterFile {
        path: "Makefile",
        content: MAKEFILE,
    },
    CookiecutterFile {
        path: "cave-pipeline.yaml",
        content: CAVE_PIPELINE_YAML,
    },
    CookiecutterFile {
        path: "catalog-info.yaml",
        content: CATALOG_INFO_YAML,
    },
    CookiecutterFile {
        path: "README.md",
        content: README_MD,
    },
    CookiecutterFile {
        path: ".cave/component.yaml",
        content: CAVE_COMPONENT_YAML,
    },
];

// ---------------------------------------------------------------------------
// File-content constants
// ---------------------------------------------------------------------------

/// `pip.conf` — pinned to cave-registry. Public PyPI is intentionally NOT set
/// as `extra-index-url` to guarantee every dependency is mirrored by the
/// sovereign registry. Operators who need a fall-through to public PyPI can
/// add `extra-index-url = https://pypi.org/simple` at their own risk.
const PIP_CONF: &str = "[global]\n\
index-url = https://{{ cookiecutter.cave_registry_host }}/api/registry/pypi/simple/\n\
trusted-host = {{ cookiecutter.cave_registry_host }}\n\
timeout = 60\n\
no-cache-dir = false\n\
disable-pip-version-check = true\n\
\n\
[install]\n\
# All installs are sourced from cave-registry. Uncomment the next line only\n\
# if you have approval from platform-security to fall through to public PyPI.\n\
# extra-index-url = https://pypi.org/simple\n\
";

/// `uv.toml` — same index pinning for uv (modern Python package manager).
const UV_TOML: &str = "[[index]]\n\
name = \"cave-registry\"\n\
url = \"https://{{ cookiecutter.cave_registry_host }}/api/registry/pypi/simple/\"\n\
default = true\n\
\n\
# Public PyPI is intentionally omitted. Add only with platform-security sign-off:\n\
# [[index]]\n\
# name = \"pypi\"\n\
# url = \"https://pypi.org/simple\"\n\
\n\
[pip]\n\
index-url = \"https://{{ cookiecutter.cave_registry_host }}/api/registry/pypi/simple/\"\n\
";

const PYPROJECT_TOML: &str = "[project]\n\
name = \"{{ cookiecutter.package_name }}\"\n\
version = \"0.1.0\"\n\
description = \"{{ cookiecutter.description }}\"\n\
requires-python = \">={{ cookiecutter.python_version }}\"\n\
readme = \"README.md\"\n\
license = { text = \"Proprietary\" }\n\
authors = [{ name = \"{{ cookiecutter.owner }}\" }]\n\
dependencies = [\n\
    \"fastapi>=0.115\",\n\
    \"uvicorn[standard]>=0.30\",\n\
    \"pydantic>=2.7\",\n\
    \"pydantic-settings>=2.3\",\n\
    \"httpx>=0.27\",\n\
    \"structlog>=24.1\",\n\
    \"opentelemetry-api>=1.27\",\n\
    \"opentelemetry-sdk>=1.27\",\n\
    \"opentelemetry-instrumentation-fastapi>=0.48b0\",\n\
]\n\
\n\
[project.optional-dependencies]\n\
dev = [\n\
    \"pytest>=8.3\",\n\
    \"pytest-asyncio>=0.24\",\n\
    \"pytest-cov>=5.0\",\n\
    \"ruff>=0.6\",\n\
    \"mypy>=1.11\",\n\
    \"bandit>=1.7\",\n\
    \"types-requests\",\n\
]\n\
\n\
[build-system]\n\
requires = [\"hatchling\"]\n\
build-backend = \"hatchling.build\"\n\
\n\
[tool.hatch.build.targets.wheel]\n\
packages = [\"src/{{ cookiecutter.package_name }}\"]\n\
\n\
# ------ CAVE sovereign index: all installs resolve via cave-registry -------\n\
[[tool.uv.index]]\n\
name = \"cave-registry\"\n\
url = \"https://{{ cookiecutter.cave_registry_host }}/api/registry/pypi/simple/\"\n\
default = true\n\
\n\
[tool.pytest.ini_options]\n\
asyncio_mode = \"auto\"\n\
addopts = \"-ra -q --cov=src/{{ cookiecutter.package_name }} --cov-report=term-missing\"\n\
testpaths = [\"tests\"]\n\
\n\
[tool.ruff]\n\
line-length = 100\n\
target-version = \"py312\"\n\
\n\
[tool.ruff.lint]\n\
select = [\"E\", \"F\", \"W\", \"I\", \"B\", \"UP\", \"SIM\", \"ASYNC\", \"S\"]\n\
ignore = [\"S101\"]\n\
\n\
[tool.mypy]\n\
python_version = \"{{ cookiecutter.python_version }}\"\n\
strict = true\n\
warn_unused_ignores = true\n\
\n\
[tool.bandit]\n\
exclude_dirs = [\"tests\", \".venv\"]\n\
";

const PACKAGE_INIT: &str = "\"\"\"{{ cookiecutter.description }}.\"\"\"\n\
\n\
__version__ = \"0.1.0\"\n\
";

const MAIN_PY: &str = "\"\"\"FastAPI application entrypoint for {{ cookiecutter.service_name }}.\"\"\"\n\
\n\
from __future__ import annotations\n\
\n\
import structlog\n\
from fastapi import FastAPI\n\
from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor\n\
\n\
from {{ cookiecutter.package_name }}.config import Settings\n\
from {{ cookiecutter.package_name }}.routes import health\n\
\n\
log = structlog.get_logger(__name__)\n\
\n\
\n\
def create_app() -> FastAPI:\n\
    settings = Settings()\n\
    app = FastAPI(\n\
        title=\"{{ cookiecutter.service_name }}\",\n\
        version=\"0.1.0\",\n\
        description=\"{{ cookiecutter.description }}\",\n\
    )\n\
    app.include_router(health.router, tags=[\"health\"])\n\
    FastAPIInstrumentor.instrument_app(app)\n\
    log.info(\"service.started\", service=\"{{ cookiecutter.service_name }}\", env=settings.environment)\n\
    return app\n\
\n\
\n\
app = create_app()\n\
";

const CONFIG_PY: &str = "\"\"\"Typed configuration loaded from environment for {{ cookiecutter.service_name }}.\"\"\"\n\
\n\
from __future__ import annotations\n\
\n\
from pydantic_settings import BaseSettings, SettingsConfigDict\n\
\n\
\n\
class Settings(BaseSettings):\n\
    model_config = SettingsConfigDict(env_prefix=\"APP_\", env_file=\".env\", extra=\"ignore\")\n\
\n\
    environment: str = \"dev\"\n\
    log_level: str = \"INFO\"\n\
    host: str = \"0.0.0.0\"\n\
    port: int = 8080\n\
";

const HEALTH_PY: &str = "\"\"\"Liveness and readiness endpoints.\"\"\"\n\
\n\
from __future__ import annotations\n\
\n\
from fastapi import APIRouter\n\
\n\
router = APIRouter()\n\
\n\
\n\
@router.get(\"/healthz\")\n\
async def healthz() -> dict[str, str]:\n\
    return {\"status\": \"ok\", \"service\": \"{{ cookiecutter.service_name }}\"}\n\
\n\
\n\
@router.get(\"/readyz\")\n\
async def readyz() -> dict[str, str]:\n\
    return {\"status\": \"ready\"}\n\
";

const TEST_HEALTH_PY: &str = "\"\"\"Smoke tests for the health endpoints.\"\"\"\n\
\n\
from fastapi.testclient import TestClient\n\
\n\
from {{ cookiecutter.package_name }}.main import app\n\
\n\
client = TestClient(app)\n\
\n\
\n\
def test_healthz() -> None:\n\
    resp = client.get(\"/healthz\")\n\
    assert resp.status_code == 200\n\
    assert resp.json()[\"status\"] == \"ok\"\n\
\n\
\n\
def test_readyz() -> None:\n\
    resp = client.get(\"/readyz\")\n\
    assert resp.status_code == 200\n\
    assert resp.json()[\"status\"] == \"ready\"\n\
";

const DOCKERFILE: &str = "# syntax=docker/dockerfile:1.7\n\
# Multi-stage build — the builder resolves dependencies through the CAVE\n\
# sovereign PyPI mirror only. No public PyPI traffic, no dependency\n\
# confusion risk.\n\
\n\
ARG PYTHON_VERSION={{ cookiecutter.python_version }}\n\
ARG CAVE_REGISTRY_HOST={{ cookiecutter.cave_registry_host }}\n\
\n\
FROM python:${PYTHON_VERSION}-slim AS builder\n\
ARG CAVE_REGISTRY_HOST\n\
ENV PIP_INDEX_URL=https://${CAVE_REGISTRY_HOST}/api/registry/pypi/simple/ \\\n\
    PIP_TRUSTED_HOST=${CAVE_REGISTRY_HOST} \\\n\
    PIP_DISABLE_PIP_VERSION_CHECK=1 \\\n\
    PIP_NO_CACHE_DIR=0 \\\n\
    UV_INDEX_URL=https://${CAVE_REGISTRY_HOST}/api/registry/pypi/simple/\n\
\n\
RUN pip install --upgrade pip uv\n\
WORKDIR /app\n\
COPY pyproject.toml pip.conf uv.toml ./\n\
COPY src ./src\n\
RUN uv pip install --system --no-deps -e . && \\\n\
    uv pip install --system .\n\
\n\
FROM python:${PYTHON_VERSION}-slim AS runtime\n\
ARG CAVE_REGISTRY_HOST\n\
ENV PIP_INDEX_URL=https://${CAVE_REGISTRY_HOST}/api/registry/pypi/simple/ \\\n\
    PIP_TRUSTED_HOST=${CAVE_REGISTRY_HOST} \\\n\
    PYTHONUNBUFFERED=1 \\\n\
    PYTHONDONTWRITEBYTECODE=1\n\
\n\
RUN useradd --system --uid 10001 --shell /usr/sbin/nologin app\n\
WORKDIR /app\n\
COPY --from=builder /usr/local/lib/python3*/site-packages /usr/local/lib/python3*/site-packages\n\
COPY --from=builder /usr/local/bin/uvicorn /usr/local/bin/uvicorn\n\
COPY src ./src\n\
COPY pip.conf /etc/pip.conf\n\
USER app\n\
EXPOSE 8080\n\
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \\\n\
    CMD python -c \"import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz').read()\" || exit 1\n\
CMD [\"uvicorn\", \"{{ cookiecutter.package_name }}.main:app\", \"--host\", \"0.0.0.0\", \"--port\", \"8080\"]\n\
";

const DOCKERIGNORE: &str = ".git\n\
.gitignore\n\
.venv\n\
__pycache__\n\
*.pyc\n\
*.pyo\n\
.pytest_cache\n\
.mypy_cache\n\
.ruff_cache\n\
.coverage\n\
htmlcov\n\
dist\n\
build\n\
*.egg-info\n\
.env\n\
.env.*\n\
tests\n\
";

const GITIGNORE: &str = "# Python\n\
__pycache__/\n\
*.py[cod]\n\
*$py.class\n\
*.so\n\
.Python\n\
.venv/\n\
venv/\n\
env/\n\
build/\n\
dist/\n\
*.egg-info/\n\
.installed.cfg\n\
\n\
# Testing / coverage\n\
.pytest_cache/\n\
.coverage\n\
.coverage.*\n\
htmlcov/\n\
.tox/\n\
.cache/\n\
\n\
# Type checking / linting\n\
.mypy_cache/\n\
.ruff_cache/\n\
.dmypy.json\n\
\n\
# Secrets / env\n\
.env\n\
.env.*\n\
\n\
# IDEs\n\
.idea/\n\
.vscode/\n\
*.swp\n\
.DS_Store\n\
";

const MAKEFILE: &str = ".DEFAULT_GOAL := help\n\
.PHONY: help install lint type test security build run clean\n\
\n\
PY ?= python\n\
UV ?= uv\n\
PACKAGE := {{ cookiecutter.package_name }}\n\
\n\
help:\n\
\t@echo \"Targets: install lint type test security build run clean\"\n\
\n\
install: ## Resolve dependencies through cave-registry\n\
\t$(UV) sync --all-extras\n\
\n\
lint: ## ruff format + check\n\
\t$(UV) run ruff format --check .\n\
\t$(UV) run ruff check .\n\
\n\
type: ## mypy strict\n\
\t$(UV) run mypy src\n\
\n\
test: ## pytest with coverage\n\
\t$(UV) run pytest\n\
\n\
security: ## bandit SAST\n\
\t$(UV) run bandit -c pyproject.toml -r src\n\
\n\
build: ## build wheel\n\
\t$(UV) build\n\
\n\
run: ## local uvicorn reload\n\
\t$(UV) run uvicorn $(PACKAGE).main:app --reload --port 8080\n\
\n\
clean:\n\
\trm -rf build dist .pytest_cache .mypy_cache .ruff_cache .coverage htmlcov\n\
";

const CAVE_PIPELINE_YAML: &str = "apiVersion: cave.caveplatform.dev/v1\n\
kind: Pipeline\n\
metadata:\n\
  name: {{ cookiecutter.service_name }}-ci\n\
  owner: {{ cookiecutter.owner }}\n\
  labels:\n\
    language: python\n\
    runtime: fastapi\n\
    sovereign-index: cave-registry\n\
spec:\n\
  triggers:\n\
    - type: git\n\
      branches: [\"main\", \"release/*\"]\n\
    - type: pullRequest\n\
  params:\n\
    - name: imageTag\n\
      value: \"{{ '{{' }} git.sha {{ '}}' }}\"\n\
    - name: caveRegistry\n\
      value: \"{{ cookiecutter.cave_registry_host }}\"\n\
  tasks:\n\
    - name: clone\n\
      ref: git-clone\n\
      params: { url: \"$(params.repoUrl)\", revision: \"$(params.revision)\" }\n\
    - name: uv-sync\n\
      ref: uv-sync              # cave-pipelines catalog (python task — pending)\n\
      runAfter: [clone]\n\
      params: { index: \"$(params.caveRegistry)\" }\n\
    - name: lint\n\
      ref: ruff\n\
      runAfter: [uv-sync]\n\
    - name: type\n\
      ref: mypy\n\
      runAfter: [uv-sync]\n\
    - name: test\n\
      ref: pytest\n\
      runAfter: [uv-sync]\n\
    - name: security\n\
      ref: bandit\n\
      runAfter: [uv-sync]\n\
    - name: image\n\
      ref: buildah\n\
      runAfter: [lint, type, test, security]\n\
      params:\n\
        IMAGE: \"{{ cookiecutter.image_registry }}/{{ cookiecutter.owner }}/{{ cookiecutter.service_name }}:$(params.imageTag)\"\n\
        BUILD_EXTRA_ARGS: \"--build-arg CAVE_REGISTRY_HOST={{ cookiecutter.cave_registry_host }}\"\n\
    - name: scan-image\n\
      ref: trivy-scanner\n\
      runAfter: [image]\n\
    - name: sign\n\
      ref: sign-image           # cave-sign (cosign replacement)\n\
      runAfter: [scan-image]\n\
    - name: sbom\n\
      ref: sbom-generate        # cave-sbom\n\
      runAfter: [scan-image]\n\
    - name: publish\n\
      ref: pypi-publish         # cave-pipelines catalog (python task — pending)\n\
      runAfter: [sign, sbom]\n\
      when: \"$(params.revision) =~ /^refs\\\\/tags\\\\/v/\"\n\
      params: { index: \"$(params.caveRegistry)\" }\n\
";

const CATALOG_INFO_YAML: &str = "apiVersion: cave.caveplatform.dev/v1\n\
kind: Component\n\
metadata:\n\
  name: {{ cookiecutter.service_name }}\n\
  description: {{ cookiecutter.description }}\n\
  annotations:\n\
    cave.caveplatform.dev/language: python\n\
    cave.caveplatform.dev/framework: fastapi\n\
    cave.caveplatform.dev/scaffold-template: python-fastapi-service\n\
  tags:\n\
    - python\n\
    - fastapi\n\
    - microservice\n\
spec:\n\
  type: service\n\
  lifecycle: experimental\n\
  owner: {{ cookiecutter.owner }}\n\
  system: platform\n\
  providesApis:\n\
    - {{ cookiecutter.service_name }}-http\n\
";

const CAVE_COMPONENT_YAML: &str = "# Local component manifest read by cave-ctl and cave-admission.\n\
name: {{ cookiecutter.service_name }}\n\
owner: {{ cookiecutter.owner }}\n\
kind: service\n\
lifecycle: experimental\n\
language: python\n\
runtime: fastapi\n\
pipeline: cave-pipeline.yaml\n\
registry:\n\
  pypi: https://{{ cookiecutter.cave_registry_host }}/api/registry/pypi/simple/\n\
  oci: {{ cookiecutter.image_registry }}\n\
policies:\n\
  - require-pqc-signature: false   # phase 3 of ADR-132 flips this to true\n\
  - require-sbom: true\n\
";

const README_MD: &str = "# {{ cookiecutter.service_name }}\n\
\n\
{{ cookiecutter.description }}\n\
\n\
Scaffolded from `python-fastapi-service` (cave-scaffold). All Python\n\
dependencies resolve through the **sovereign CAVE artifact registry**\n\
(`{{ cookiecutter.cave_registry_host }}`) — not public PyPI.\n\
\n\
## Quick start\n\
\n\
```bash\n\
make install   # uv sync via cave-registry\n\
make run       # uvicorn reload on :8080\n\
make lint type test security\n\
```\n\
\n\
## Index pinning\n\
\n\
Look at `pip.conf`, `uv.toml`, `pyproject.toml` (`[[tool.uv.index]]`) and the\n\
`Dockerfile` — all four independently pin the index to cave-registry so no\n\
single config drift can leak the build to public PyPI.\n\
\n\
## CI/CD\n\
\n\
`cave-pipeline.yaml` is picked up automatically by cave-pipelines when the\n\
repo is registered (via the scaffold `ci:create-pipeline` step). It runs\n\
uv-sync → ruff → mypy → pytest → bandit → buildah → trivy → sign → sbom →\n\
pypi-publish (on tag).\n\
\n\
## Catalog\n\
\n\
`catalog-info.yaml` registers this service in cave-portal. See it under\n\
`/portal/catalog/{{ cookiecutter.service_name }}`.\n\
";

// ---------------------------------------------------------------------------
// Store seeding
// ---------------------------------------------------------------------------

/// Build the set of `Template` records that should be seeded into a fresh
/// `ScaffoldStore`. Callable from synchronous contexts — no awaits.
pub fn seeded_templates() -> Vec<Template> {
    builtin_cookiecutter_templates()
        .iter()
        .map(to_template)
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_values() -> HashMap<String, String> {
        let mut v = HashMap::new();
        v.insert("service_name".to_string(), "billing-api".to_string());
        v.insert("package_name".to_string(), "billing_api".to_string());
        v.insert("description".to_string(), "Billing API".to_string());
        v.insert("owner".to_string(), "payments".to_string());
        v.insert("python_version".to_string(), "3.12".to_string());
        v.insert(
            "cave_registry_host".to_string(),
            "cave-registry.cave.caveplatform.dev".to_string(),
        );
        v.insert(
            "image_registry".to_string(),
            "cave-registry.cave.caveplatform.dev".to_string(),
        );
        v
    }

    #[test]
    fn substitute_compatible with_cookiecutter_vars() {
        let mut values = HashMap::new();
        values.insert("name".to_string(), "billing".to_string());
        let out = substitute("service-{{ cookiecutter.name }}-v1", &values);
        assert_eq!(out, "service-billing-v1");
    }

    #[test]
    fn substitute_leaves_unknown_vars() {
        let values = HashMap::new();
        let out = substitute("hello {{ cookiecutter.missing }}", &values);
        assert!(out.contains("{{ cookiecutter.missing }}"));
    }

    #[test]
    fn substitute_ignores_non_cookiecutter_namespace() {
        let mut values = HashMap::new();
        values.insert("foo".to_string(), "bar".to_string());
        let out = substitute("{{ foo }}", &values);
        assert_eq!(out, "{{ foo }}"); // non-cookiecutter namespace preserved
    }

    #[test]
    fn python_fastapi_template_has_pip_conf_pointing_to_cave_registry() {
        let tpl = python_fastapi_service();
        let pip_conf = tpl
            .files
            .iter()
            .find(|f| f.path == "pip.conf")
            .expect("pip.conf must be present");
        assert!(pip_conf.content.contains("cave_registry_host"));
        assert!(pip_conf.content.contains("/api/registry/pypi/simple/"));
        assert!(
            !pip_conf
                .content
                .lines()
                .any(|l| l.trim_start().starts_with("extra-index-url = https://pypi.org")),
            "public PyPI must not be enabled by default"
        );
    }

    #[test]
    fn python_fastapi_template_renders_all_files() {
        let tpl = python_fastapi_service();
        let values = sample_values();
        let files = render(&tpl, &values);
        assert_eq!(files.len(), tpl.files.len());
        // package directory path is substituted
        assert!(files.iter().any(|(p, _)| p == "src/billing_api/main.py"));
        // pip.conf content has host expanded
        let (_, pip_content) = files
            .iter()
            .find(|(p, _)| p == "pip.conf")
            .expect("pip.conf");
        assert!(pip_content.contains("cave-registry.cave.caveplatform.dev"));
        assert!(!pip_content.contains("{{ cookiecutter.cave_registry_host }}"));
    }

    #[test]
    fn seeded_templates_contains_python_fastapi() {
        let seeds = seeded_templates();
        assert!(seeds.iter().any(|t| t.name == "python-fastapi-service"));
        let t = seeds.iter().find(|t| t.name == "python-fastapi-service").unwrap();
        // Must have the four pipeline steps.
        assert_eq!(t.steps.len(), 4);
        let step_ids: Vec<&str> = t.steps.iter().map(|s| s.action.as_str()).collect();
        assert!(step_ids.contains(&"cookiecutter:render"));
        assert!(step_ids.contains(&"publish:github"));
        assert!(step_ids.contains(&"ci:create-pipeline"));
        assert!(step_ids.contains(&"catalog:register"));
    }

    #[test]
    fn pyproject_pins_uv_index_to_cave_registry() {
        let tpl = python_fastapi_service();
        let pyproject = tpl
            .files
            .iter()
            .find(|f| f.path == "pyproject.toml")
            .unwrap();
        assert!(pyproject.content.contains("[[tool.uv.index]]"));
        assert!(pyproject.content.contains("name = \"cave-registry\""));
        assert!(pyproject
            .content
            .contains("/api/registry/pypi/simple/"));
    }

    #[test]
    fn dockerfile_inherits_cave_registry_env() {
        let tpl = python_fastapi_service();
        let dockerfile = tpl
            .files
            .iter()
            .find(|f| f.path == "Dockerfile")
            .unwrap();
        assert!(dockerfile.content.contains("PIP_INDEX_URL"));
        assert!(dockerfile.content.contains("UV_INDEX_URL"));
        assert!(dockerfile.content.contains("CAVE_REGISTRY_HOST"));
        // Default host baked in so the image builds without user overrides.
        assert!(dockerfile
            .content
            .contains("{{ cookiecutter.cave_registry_host }}"));
    }
}
