use crate::engine::{ScanError, Scanner};
use crate::models::{Finding, FindingCategory, Confidence, ScanKind, ScanRequest, ScanTarget, Severity, Ecosystem};
use async_trait::async_trait;
use std::collections::HashSet;

pub struct NamespaceScanner;

impl NamespaceScanner {
    fn get_top_packages(&self, ecosystem: &Ecosystem) -> HashSet<String> {
        match ecosystem {
            Ecosystem::PyPI => vec![
                "requests", "numpy", "pandas", "django", "flask",
                "pytest", "sqlalchemy", "pillow", "scipy", "matplotlib",
                "celery", "pyyaml", "jinja2", "cryptography", "boto3",
                "redis", "pymongo", "psycopg2", "httpx", "fastapi",
                "aiohttp", "beautifulsoup4", "lxml", "click", "six",
                "setuptools", "wheel", "pip", "virtualenv", "tox",
                "black", "pylint", "flake8", "mypy", "pytest-cov",
                "coverage", "mock", "faker", "factory_boy", "hypothesis",
                "attrs", "dataclasses", "typing_extensions", "pathlib",
                "inspect", "logging", "sys", "os", "json",
                "xml", "csv", "datetime", "time", "random",
            ],
            Ecosystem::Npm => vec![
                "react", "angular", "vue", "svelte", "express",
                "next", "nuxt", "gatsby", "webpack", "babel",
                "typescript", "eslint", "prettier", "jest", "mocha",
                "chai", "lodash", "moment", "axios", "node-fetch",
                "socket.io", "passport", "bcrypt", "jsonwebtoken", "dotenv",
                "express-session", "cors", "helmet", "validator", "joi",
                "yup", "formik", "redux", "mobx", "zustand",
                "react-router", "next-auth", "prisma", "sequelize", "typeorm",
                "graphql", "apollo", "grpc", "websocket", "ws",
            ],
            Ecosystem::Maven => vec![
                "junit", "testng", "mockito", "hamcrest", "assertj",
                "log4j", "slf4j", "logback", "commons-lang", "commons-io",
                "commons-collections", "commons-dbcp", "commons-pool",
                "google-guava", "gson", "jackson", "fastjson", "protobuf",
                "spring-boot", "spring-data", "hibernate", "mybatis",
                "apache-commons", "netty", "grpc-java", "h2", "mysql",
                "postgresql", "oracle", "mssql", "mariadb", "elasticsearch",
                "kafka", "rabbitmq", "redis", "memcached", "cassandra",
            ],
            Ecosystem::RubyGems => vec![
                "rails", "sinatra", "rack", "bundler", "gem",
                "rspec", "minitest", "capybara", "selenium", "cucumber",
                "activesupport", "activerecord", "activemodel", "actionview",
                "sequel", "datamapper", "mongoid", "redis", "sidekiq",
                "devise", "pundit", "cancancan", "bcrypt", "jwt",
                "dotenv", "config", "figaro", "settingslogic", "rails_config",
                "puma", "unicorn", "thin", "webrick", "passenger",
                "nginx", "apache", "haproxy", "pry", "byebug",
                "ruby-debug", "binding_of_caller", "better_errors",
            ],
            Ecosystem::Cargo => vec![
                "tokio", "async-std", "actix", "actix-web", "rocket",
                "axum", "warp", "hyper", "reqwest", "http",
                "serde", "serde_json", "toml", "yaml", "ron",
                "regex", "lazy_static", "once_cell", "thiserror", "anyhow",
                "log", "tracing", "env_logger", "chrono", "time",
                "uuid", "rand", "sha2", "hmac", "aes",
                "rsa", "x509", "rustls", "openssl", "libsodium",
            ],
            Ecosystem::Go => vec![
                "database/sql", "encoding/json", "fmt", "io", "net",
                "net/http", "os", "path", "path/filepath", "regexp",
                "sort", "strings", "sync", "time", "crypto",
                "crypto/sha256", "encoding/base64", "encoding/hex", "hash",
                "golang.org/x/crypto", "golang.org/x/sys", "golang.org/x/net",
                "github.com/gorilla/mux", "github.com/gin-gonic/gin",
                "github.com/labstack/echo", "github.com/beego/beego",
                "github.com/gorm/gorm", "github.com/jinzhu/copier",
                "github.com/streadway/amqp", "github.com/go-redis/redis",
            ],
            Ecosystem::NuGet => vec![
                "Newtonsoft.Json", "System.Json", "System.Text.Json",
                "AutoMapper", "Dapper", "Entity.Framework", "NHibernate",
                "Serilog", "NLog", "log4net", "Castle.Core",
                "Xunit", "NUnit", "Moq", "FluentAssertions",
                "Microsoft.Extensions.DependencyInjection", "Autofac",
                "Nancy", "ServiceStack", "Restsharp", "HttpClientFactory",
            ],
            Ecosystem::Composer => vec![
                "symfony/console", "symfony/http-foundation", "laravel/framework",
                "doctrine/orm", "zendframework/zend-mvc", "guzzlehttp/guzzle",
                "monolog/monolog", "phpunit/phpunit", "phpspec/phpspec",
                "behat/behat", "mockery/mockery", "phpdotenv",
                "league/oauth2-server", "firebase/jwt", "paragonie/halite",
            ],
            Ecosystem::Oci => vec![
                "alpine", "ubuntu", "debian", "centos", "fedora",
                "busybox", "scratch", "golang", "python", "node",
                "rust", "ruby", "php", "java", "dotnet",
                "nginx", "apache", "httpd", "postgres", "mysql",
                "redis", "mongodb", "elasticsearch", "cassandra", "kafka",
            ],
        }
        .into_iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn levenshtein_distance(s1: &str, s2: &str) -> usize {
        let len1 = s1.len();
        let len2 = s2.len();
        let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

        for i in 0..=len1 {
            matrix[i][0] = i;
        }
        for j in 0..=len2 {
            matrix[0][j] = j;
        }

        for (i, c1) in s1.chars().enumerate() {
            for (j, c2) in s2.chars().enumerate() {
                let cost = if c1 == c2 { 0 } else { 1 };
                matrix[i + 1][j + 1] = std::cmp::min(
                    std::cmp::min(
                        matrix[i][j + 1] + 1,
                        matrix[i + 1][j] + 1,
                    ),
                    matrix[i][j] + cost,
                );
            }
        }

        matrix[len1][len2]
    }

    fn is_typosquat(&self, ecosystem: &Ecosystem, name: &str) -> bool {
        let top_packages = self.get_top_packages(ecosystem);

        // If name is already in top packages, it's not a typosquat
        if top_packages.contains(&name.to_lowercase()) {
            return false;
        }

        // Check Levenshtein distance <= 1 to any top package
        for pkg in &top_packages {
            let distance = Self::levenshtein_distance(&name.to_lowercase(), pkg);
            if distance <= 1 {
                return true;
            }
        }

        false
    }
}

#[async_trait::async_trait]
impl Scanner for NamespaceScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Namespace
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::PackageName { ecosystem, name } => {
                let mut findings = vec![];

                if self.is_typosquat(ecosystem, name) {
                    let mut f = Finding::new(
                        "NS-001".to_string(),
                        "Potential namespace confusion / typosquat".to_string(),
                        FindingCategory::Typosquat,
                        Severity::High,
                        format!("Package name '{}' may be a typosquat of legitimate package", name),
                        "This package name is suspiciously similar to a popular package in this ecosystem".to_string(),
                    );
                    f.location.package = Some(name.clone());
                    f.remediation = Some(format!("Verify the package '{}' is from a trusted source before installing", name));
                    f.confidence = Confidence::High;
                    findings.push(f);
                }

                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest("Expected PackageName target".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(NamespaceScanner::levenshtein_distance("requests", "requets"), 1);
        assert_eq!(NamespaceScanner::levenshtein_distance("requests", "requests"), 0);
        assert_eq!(NamespaceScanner::levenshtein_distance("flask", "falsk"), 2);
    }

    #[tokio::test]
    async fn test_namespace_typosquat_detection() {
        let scanner = NamespaceScanner;
        let req = ScanRequest {
            kind: ScanKind::Namespace,
            target: ScanTarget::PackageName {
                ecosystem: Ecosystem::PyPI,
                name: "requets".to_string(), // typosquat of requests
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "NS-001"));
    }

    #[tokio::test]
    async fn test_namespace_legitimate_package() {
        let scanner = NamespaceScanner;
        let req = ScanRequest {
            kind: ScanKind::Namespace,
            target: ScanTarget::PackageName {
                ecosystem: Ecosystem::PyPI,
                name: "requests".to_string(), // legitimate package
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.is_empty());
    }

    #[tokio::test]
    async fn test_namespace_another_legitimate() {
        let scanner = NamespaceScanner;
        let req = ScanRequest {
            kind: ScanKind::Namespace,
            target: ScanTarget::PackageName {
                ecosystem: Ecosystem::PyPI,
                name: "urllib3".to_string(), // legitimate package
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.is_empty());
    }
}
