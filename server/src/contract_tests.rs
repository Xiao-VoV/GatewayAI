//! OpenAPI 机器契约 lint（普通单元测试，不需要数据库）。
//!
//! 目的：CI 每次跑 `cargo test` 时校验各模块 `接口契约.yaml` 的结构、引用、
//! 操作集合、成功响应头、错误码覆盖与敏感字段约束，避免契约漂移。

#![cfg(test)]

use serde_yaml::Value;

const IDENTITY_YAML: &str = include_str!("../../contracts/identity.yaml");
const PROVIDERS_YAML: &str = include_str!("../../contracts/providers.yaml");
const ACCESS_YAML: &str = include_str!("../../contracts/access.yaml");
const POLICY_YAML: &str = include_str!("../../contracts/policy.yaml");
const ROUTING_YAML: &str = include_str!("../../contracts/routing.yaml");
const PROXY_YAML: &str = include_str!("../../contracts/proxy.yaml");
const METERING_YAML: &str = include_str!("../../contracts/metering.yaml");
const OBSERVABILITY_YAML: &str = include_str!("../../contracts/observability.yaml");
const CATALOG_YAML: &str = include_str!("../../contracts/catalog.yaml");
const SETTINGS_YAML: &str = include_str!("../../contracts/settings.yaml");

fn load(raw: &str) -> Value {
    serde_yaml::from_str(raw).expect("契约 yaml 必须可解析")
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let path = reference.strip_prefix("#/")?;
    let mut current = root;
    for segment in path.split('/') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn collect_refs(value: &Value, refs: &mut Vec<String>) {
    match value {
        Value::Mapping(map) => {
            for (key, child) in map {
                if key.as_str() == Some("$ref")
                    && let Some(reference) = child.as_str()
                {
                    refs.push(reference.to_string());
                }
                collect_refs(child, refs);
            }
        }
        Value::Sequence(items) => {
            for item in items {
                collect_refs(item, refs);
            }
        }
        _ => {}
    }
}

fn operations(doc: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    let paths = doc.get("paths").and_then(Value::as_mapping).expect("paths");
    for (path, item) in paths {
        let Some(path) = path.as_str() else { continue };
        let Some(item) = item.as_mapping() else {
            continue;
        };
        for (method, operation) in item {
            let Some(method) = method.as_str() else {
                continue;
            };
            if matches!(method, "get" | "post" | "put" | "patch" | "delete") {
                out.push((path.to_string(), method.to_string(), operation.clone()));
            }
        }
    }
    out
}

fn response_codes(operation: &Value) -> Vec<String> {
    operation
        .get("responses")
        .and_then(Value::as_mapping)
        .map(|responses| {
            responses
                .keys()
                .filter_map(|key| key.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn success_responses(operation: &Value) -> Vec<Value> {
    let Some(responses) = operation.get("responses").and_then(Value::as_mapping) else {
        return Vec::new();
    };
    responses
        .iter()
        .filter(|(code, _)| code.as_str().is_some_and(|code| code.starts_with('2')))
        .map(|(_, response)| response.clone())
        .collect()
}

fn has_header(root: &Value, response: &Value, header: &str, expected_ref: &str) -> bool {
    let Some(headers) = response.get("headers").and_then(Value::as_mapping) else {
        return false;
    };
    headers.iter().any(|(name, value)| {
        if name
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(header))
        {
            return true;
        }
        value
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|reference| {
                reference.ends_with(expected_ref) && resolve(root, reference).is_some()
            })
    })
}

fn assert_common_contract(name: &str, doc: &Value) {
    let version = doc
        .get("openapi")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(version, "3.1.0", "{name}: openapi 版本必须是 3.1.0");

    let mut refs = Vec::new();
    collect_refs(doc, &mut refs);
    assert!(!refs.is_empty(), "{name}: 应包含 $ref");
    for reference in refs {
        assert!(
            resolve(doc, &reference).is_some(),
            "{name}: 悬空 $ref {reference}"
        );
    }

    let operations = operations(doc);
    assert!(!operations.is_empty(), "{name}: 缺少 paths");

    let mut operation_ids = std::collections::BTreeSet::new();
    for (path, method, operation) in &operations {
        let operation_id = operation
            .get("operationId")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name}: {method} {path} 缺 operationId"));
        assert!(
            operation_ids.insert(operation_id.to_string()),
            "{name}: operationId 重复 {operation_id}"
        );
        assert!(
            operation.get("summary").is_some(),
            "{name}: {method} {path} 缺 summary"
        );

        let codes = response_codes(operation);
        assert!(
            codes.iter().any(|code| code.starts_with('2')),
            "{name}: {method} {path} 缺 2xx 响应"
        );
        for response in success_responses(operation) {
            assert!(
                has_header(doc, &response, "X-Request-Id", "RequestId"),
                "{name}: {method} {path} 成功响应缺 x-request-id"
            );
            assert!(
                has_header(doc, &response, "Cache-Control", "NoStore"),
                "{name}: {method} {path} 成功响应缺 cache-control"
            );
        }
    }
}

#[test]
fn identity_contract_lints() {
    let doc = load(IDENTITY_YAML);
    assert_common_contract("identity", &doc);

    let names: Vec<(String, String)> = operations(&doc)
        .into_iter()
        .map(|(path, method, _)| (path, method))
        .collect();
    for expected in [
        ("/api/admin/auth/setup", "get"),
        ("/api/admin/auth/setup", "post"),
        ("/api/admin/auth/login", "post"),
        ("/api/admin/auth/me", "get"),
        ("/api/admin/auth/logout", "post"),
    ] {
        assert!(
            names
                .iter()
                .any(|(path, method)| path == expected.0 && method == expected.1),
            "identity: 缺少 {expected:?}"
        );
    }

    let admin_user = resolve(&doc, "#/components/schemas/AdminUser").expect("AdminUser");
    let properties = admin_user
        .get("properties")
        .and_then(Value::as_mapping)
        .expect("AdminUser.properties");
    for forbidden in ["password_hash", "password", "token"] {
        assert!(
            !properties.contains_key(Value::String(forbidden.to_string())),
            "identity: AdminUser 不得返回 {forbidden}"
        );
    }

    let problem = resolve(&doc, "#/components/schemas/Problem").expect("Problem");
    assert!(
        problem
            .get("properties")
            .and_then(|p| p.get("code"))
            .is_some()
    );
    assert!(
        problem
            .get("properties")
            .and_then(|p| p.get("errors"))
            .is_some()
    );
}

#[test]
fn providers_contract_lints() {
    let doc = load(PROVIDERS_YAML);
    assert_common_contract("providers", &doc);

    let operations = operations(&doc);
    assert_eq!(operations.len(), 15, "providers: 端点数量应为 15");

    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        for required in ["401", "503"] {
            assert!(
                codes.iter().any(|code| code == required),
                "providers: {method} {path} 缺 {required}"
            );
        }
        if matches!(method.as_str(), "post" | "patch" | "delete") {
            assert!(
                codes.iter().any(|code| code == "403"),
                "providers: 写端点 {method} {path} 缺 403（CSRF）"
            );
        }
        assert!(
            op_has_security(&doc, operation),
            "providers: {method} {path} 缺 adminSession security"
        );
    }

    let account = resolve(&doc, "#/components/schemas/ProviderAccount").expect("ProviderAccount");
    let properties = account
        .get("properties")
        .and_then(Value::as_mapping)
        .expect("ProviderAccount.properties");
    assert!(
        properties.contains_key(Value::String("api_key_hint".to_string())),
        "providers: ProviderAccount 必须返回 api_key_hint"
    );
    for forbidden in ["api_key", "credential_encrypted", "credential"] {
        assert!(
            !properties.contains_key(Value::String(forbidden.to_string())),
            "providers: ProviderAccount 不得返回 {forbidden}"
        );
    }

    let create_account =
        resolve(&doc, "#/components/schemas/CreateAccountRequest").expect("CreateAccountRequest");
    let api_key = create_account
        .get("properties")
        .and_then(|properties| properties.get("api_key"))
        .expect("api_key");
    assert_eq!(
        api_key.get("writeOnly").and_then(Value::as_bool),
        Some(true),
        "providers: api_key 必须 writeOnly"
    );

    let problem = resolve(&doc, "#/components/schemas/Problem").expect("Problem");
    assert!(
        problem
            .get("properties")
            .and_then(|p| p.get("references"))
            .is_some(),
        "providers: Problem 应支持 references"
    );
}

#[test]
fn access_contract_lints() {
    let doc = load(ACCESS_YAML);
    assert_common_contract("access", &doc);

    let operations = operations(&doc);
    assert_eq!(operations.len(), 22, "access: 端点数量应为 22");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(14),
        "access: 路径数量应为 14"
    );

    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            codes.iter().any(|code| code == "503"),
            "access: {method} {path} 缺 503"
        );
        if path.starts_with("/api/admin/") {
            assert!(
                op_has_security(&doc, operation),
                "access: {method} {path} 缺 adminSession security"
            );
            for required in ["401", "500"] {
                assert!(
                    codes.iter().any(|code| code == required),
                    "access: {method} {path} 缺 {required}"
                );
            }
            if matches!(method.as_str(), "post" | "patch" | "delete") {
                assert!(
                    codes.iter().any(|code| code == "403"),
                    "access: 管理台写端点 {method} {path} 缺 403"
                );
                assert!(
                    operation
                        .get("parameters")
                        .and_then(Value::as_sequence)
                        .is_some_and(|params| params.iter().any(|param| {
                            param
                                .get("$ref")
                                .and_then(Value::as_str)
                                .is_some_and(|reference| reference.ends_with("CsrfToken"))
                        })),
                    "access: 管理台写端点 {method} {path} 缺 CsrfToken 参数"
                );
            }
        }
        if matches!(path.as_str(), "/api/client/pair" | "/api/client/token") {
            assert!(
                codes.iter().any(|code| code == "429"),
                "access: {path} 缺 429"
            );
        }
        if path == "/api/client/device" {
            assert!(
                op_uses_scheme(&doc, operation, "clientToken"),
                "access: /api/client/device 应使用 clientToken"
            );
        }
    }

    for schema in ["User", "DeviceAdmin", "KeyAdmin", "PairingCode"] {
        let value = resolve(&doc, &format!("#/components/schemas/{schema}")).expect(schema);
        let properties = value
            .get("properties")
            .and_then(Value::as_mapping)
            .expect("schema properties");
        for forbidden in [
            "code",
            "device_secret",
            "device_secret_hash",
            "api_key",
            "key_hash",
            "token",
        ] {
            assert!(
                !properties.contains_key(Value::String(forbidden.to_string())),
                "access: {schema} 不得返回 {forbidden}"
            );
        }
    }

    let key = resolve(&doc, "#/components/schemas/KeyAdmin").expect("KeyAdmin");
    assert!(
        key.get("properties")
            .and_then(|properties| properties.get("key_prefix"))
            .is_some(),
        "access: KeyAdmin 必须返回 key_prefix"
    );

    let reset = resolve(&doc, "#/components/schemas/ResetKeyResponse").expect("ResetKeyResponse");
    assert!(
        reset
            .get("properties")
            .and_then(|properties| properties.get("api_key"))
            .is_some(),
        "access: ResetKeyResponse 应返回一次 api_key"
    );

    let pair =
        resolve(&doc, "#/components/schemas/ClientPairResponse").expect("ClientPairResponse");
    for required in ["device_secret", "api_key", "token"] {
        assert!(
            pair.get("properties")
                .and_then(|properties| properties.get(required))
                .is_some(),
            "access: ClientPairResponse 应返回 {required}"
        );
    }
}

fn op_uses_scheme(doc: &Value, operation: &Value, scheme: &str) -> bool {
    let Some(security) = operation.get("security").and_then(Value::as_sequence) else {
        return false;
    };
    security.iter().any(|entry| {
        entry
            .as_mapping()
            .is_some_and(|map| map.contains_key(Value::String(scheme.to_string())))
    }) && resolve(doc, &format!("#/components/securitySchemes/{scheme}")).is_some()
}

#[test]
fn policy_contract_lints() {
    let doc = load(POLICY_YAML);
    assert_common_contract("policy", &doc);

    let operations = operations(&doc);
    assert_eq!(operations.len(), 4, "policy: 端点数量应为 4");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(2),
        "policy: 路径数量应为 2"
    );

    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            op_has_security(&doc, operation),
            "policy: {method} {path} 缺 adminSession security"
        );
        for required in ["401", "500", "503"] {
            assert!(
                codes.iter().any(|code| code == required),
                "policy: {method} {path} 缺 {required}"
            );
        }
        if matches!(method.as_str(), "put" | "delete" | "post" | "patch") {
            assert!(
                codes.iter().any(|code| code == "403"),
                "policy: 写端点 {method} {path} 缺 403"
            );
            assert!(
                operation
                    .get("parameters")
                    .and_then(Value::as_sequence)
                    .is_some_and(|params| params.iter().any(|param| {
                        param
                            .get("$ref")
                            .and_then(Value::as_str)
                            .is_some_and(|reference| reference.ends_with("CsrfToken"))
                    })),
                "policy: 写端点 {method} {path} 缺 CsrfToken 参数"
            );
        }
    }

    let budget = resolve(&doc, "#/components/schemas/Budget").expect("Budget");
    let daily_limit = budget
        .get("properties")
        .and_then(|properties| properties.get("daily_limit"))
        .expect("daily_limit");
    assert_eq!(
        daily_limit.get("type").and_then(Value::as_str),
        Some("string"),
        "policy: daily_limit 必须是十进制字符串（避免浮点）"
    );
    let currency = budget
        .get("properties")
        .and_then(|properties| properties.get("currency"))
        .expect("currency");
    assert_eq!(
        currency
            .get("enum")
            .and_then(Value::as_sequence)
            .map(|v| v.len()),
        Some(1),
        "policy: S3 仅 CNY"
    );
    assert!(
        !operations
            .iter()
            .any(|(path, _, _)| path.starts_with("/api/client/")),
        "policy: 不应有客户端公开预算端点"
    );
}

#[test]
fn proxy_contract_lints() {
    let doc = load(PROXY_YAML);
    assert_common_contract("proxy", &doc);
    let operations = operations(&doc);
    assert_eq!(operations.len(), 5, "proxy: 端点数量应为 5");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(5),
        "proxy: 路径数量应为 5"
    );
    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            !path.starts_with("/api/"),
            "proxy: 不应包含管理台端点 {path}"
        );
        assert!(
            !path.contains("/v1/models"),
            "proxy: /v1/models 属 catalog，不应出现在 proxy 契约"
        );
        assert!(
            op_uses_scheme(&doc, operation, "virtualKey"),
            "proxy: {method} {path} 缺 virtualKey"
        );
        assert!(
            op_uses_scheme(&doc, operation, "deviceToken"),
            "proxy: {method} {path} 缺 deviceToken"
        );
        for required in ["401", "413", "415", "429", "500", "502", "504"] {
            assert!(
                codes.iter().any(|code| code == required),
                "proxy: {method} {path} 缺 {required}"
            );
        }
    }
}

#[test]
fn metering_contract_lints() {
    let doc = load(METERING_YAML);
    assert_common_contract("metering", &doc);
    let operations = operations(&doc);
    assert_eq!(operations.len(), 3, "metering: 端点数量应为 3");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(3),
        "metering: 路径数量应为 3"
    );
    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        if path.starts_with("/api/admin/") {
            assert!(
                op_has_security(&doc, operation),
                "metering: {method} {path} 缺 adminSession"
            );
            for required in ["401", "500", "503"] {
                assert!(
                    codes.iter().any(|code| code == required),
                    "metering: {method} {path} 缺 {required}"
                );
            }
        } else {
            assert_eq!(path, "/api/client/usage", "metering: 未知客户端路径 {path}");
            assert!(
                op_uses_scheme(&doc, operation, "clientApiKey"),
                "metering: {method} {path} 缺 clientApiKey"
            );
            assert!(
                op_uses_scheme(&doc, operation, "clientDeviceToken"),
                "metering: {method} {path} 缺 clientDeviceToken"
            );
            for required in ["400", "401", "403", "500", "503"] {
                assert!(
                    codes.iter().any(|code| code == required),
                    "metering: {method} {path} 缺 {required}"
                );
            }
        }
    }
}

#[test]
fn settings_contract_lints() {
    let doc = load(SETTINGS_YAML);
    assert_common_contract("settings", &doc);
    let operations = operations(&doc);
    assert_eq!(operations.len(), 2, "settings: 端点数量应为 2");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(1),
        "settings: 路径数量应为 1"
    );
    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            op_has_security(&doc, operation),
            "settings: {method} {path} 缺 adminSession security"
        );
        for required in ["401", "500", "503"] {
            assert!(
                codes.iter().any(|code| code == required),
                "settings: {method} {path} 缺 {required}"
            );
        }
        if method == "patch" {
            assert!(
                codes.iter().any(|code| code == "403"),
                "settings: patch 缺 403"
            );
        }
    }
}

#[test]
fn observability_contract_lints() {
    let doc = load(OBSERVABILITY_YAML);
    assert_common_contract("observability", &doc);
    let operations = operations(&doc);
    assert_eq!(operations.len(), 5, "observability: 端点数量应为 5");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(5),
        "observability: 路径数量应为 5"
    );
    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            path.starts_with("/api/admin/"),
            "observability: 路径应在管理台"
        );
        assert!(
            op_has_security(&doc, operation),
            "observability: {method} {path} 缺 adminSession"
        );
        for required in ["401", "500", "503"] {
            assert!(
                codes.iter().any(|code| code == required),
                "observability: {method} {path} 缺 {required}"
            );
        }
        if path.contains("{requestId}") {
            assert!(
                codes.iter().any(|code| code == "404"),
                "observability: {method} {path} 缺 404"
            );
        }
    }
}

#[test]
fn catalog_contract_lints() {
    let doc = load(CATALOG_YAML);
    assert_common_contract("catalog", &doc);
    let operations = operations(&doc);
    assert_eq!(operations.len(), 27, "catalog: 端点数量应为 27");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(16),
        "catalog: 路径数量应为 16"
    );
    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        if path.starts_with("/api/admin/") {
            assert!(
                op_uses_scheme(&doc, operation, "adminSession"),
                "catalog: 管理台 {method} {path} 缺 adminSession"
            );
            if matches!(method.as_str(), "post" | "patch" | "delete") {
                assert!(
                    codes.iter().any(|code| code == "403"),
                    "catalog: 管理台写端点 {method} {path} 缺 403"
                );
                assert!(
                    operation
                        .get("parameters")
                        .and_then(Value::as_sequence)
                        .is_some_and(|params| params.iter().any(|param| {
                            param
                                .get("$ref")
                                .and_then(Value::as_str)
                                .is_some_and(|reference| reference.ends_with("CsrfToken"))
                        })),
                    "catalog: 管理台写端点 {method} {path} 缺 CsrfToken"
                );
            }
        } else {
            assert!(
                op_uses_scheme(&doc, operation, "virtualKey"),
                "catalog: 客户端 {method} {path} 缺 virtualKey"
            );
            assert!(
                op_uses_scheme(&doc, operation, "deviceToken"),
                "catalog: 客户端 {method} {path} 缺 deviceToken"
            );
            assert!(
                codes.iter().any(|code| code == "401"),
                "catalog: 客户端 {method} {path} 缺 401"
            );
        }
    }
}

#[test]
fn routing_contract_lints() {
    let doc = load(ROUTING_YAML);
    assert_common_contract("routing", &doc);

    let operations = operations(&doc);
    assert_eq!(operations.len(), 6, "routing: 端点数量应为 6");
    assert_eq!(
        doc.get("paths")
            .and_then(Value::as_mapping)
            .map(serde_yaml::Mapping::len),
        Some(3),
        "routing: 路径数量应为 3"
    );

    for (path, method, operation) in &operations {
        let codes = response_codes(operation);
        assert!(
            op_has_security(&doc, operation),
            "routing: {method} {path} 缺 adminSession security"
        );
        for required in ["401", "500", "503"] {
            assert!(
                codes.iter().any(|code| code == required),
                "routing: {method} {path} 缺 {required}"
            );
        }
        if matches!(method.as_str(), "post" | "patch" | "delete") {
            assert!(
                codes.iter().any(|code| code == "403"),
                "routing: 写端点 {method} {path} 缺 403"
            );
            assert!(
                operation
                    .get("parameters")
                    .and_then(Value::as_sequence)
                    .is_some_and(|params| params.iter().any(|param| {
                        param
                            .get("$ref")
                            .and_then(Value::as_str)
                            .is_some_and(|reference| reference.ends_with("CsrfToken"))
                    })),
                "routing: 写端点 {method} {path} 缺 CsrfToken 参数"
            );
        }
    }
    assert!(
        !operations
            .iter()
            .any(|(path, _, _)| path.starts_with("/v1/") || path.starts_with("/api/client/")),
        "routing: 客户端 /v1/models 由 proxy/catalog 暴露，不应出现在管理契约"
    );

    let target = resolve(&doc, "#/components/schemas/TargetAdmin").expect("TargetAdmin");
    let priority = target
        .get("properties")
        .and_then(|properties| properties.get("priority"))
        .expect("priority");
    assert_eq!(priority.get("maximum").and_then(Value::as_u64), Some(1000));
    let weight = target
        .get("properties")
        .and_then(|properties| properties.get("weight"))
        .expect("weight");
    assert_eq!(weight.get("maximum").and_then(Value::as_u64), Some(100));
    let alias = resolve(&doc, "#/components/schemas/ModelAlias").expect("ModelAlias");
    assert!(
        alias
            .get("properties")
            .and_then(|p| p.get("alias"))
            .is_some(),
        "routing: ModelAlias 必须有 alias"
    );
}

fn op_has_security(doc: &Value, operation: &Value) -> bool {
    let Some(security) = operation.get("security").and_then(Value::as_sequence) else {
        return false;
    };
    security.iter().any(|entry| {
        entry
            .as_mapping()
            .is_some_and(|map| map.contains_key(Value::String("adminSession".to_string())))
    }) && resolve(doc, "#/components/securitySchemes/adminSession").is_some()
}
