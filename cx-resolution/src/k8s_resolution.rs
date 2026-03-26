use crate::helm_env_resolution::EnvVarRead;
use rustc_hash::FxHashMap;

/// An env var binding from a K8s Deployment/StatefulSet/DaemonSet manifest.
#[derive(Debug, Clone)]
pub struct K8sEnvBinding {
    /// Env var name (e.g., "PRODUCT_CATALOG_SERVICE_ADDR").
    pub var_name: String,
    /// Env var value (e.g., "productcatalogservice:3550").
    pub value: String,
    /// K8s manifest file.
    pub file: String,
    /// Line number of the `name:` field.
    pub line: u32,
    /// Deployment/StatefulSet/DaemonSet metadata.name.
    pub deployment_name: String,
}

/// A K8s Service parsed from a manifest.
#[derive(Debug, Clone)]
pub struct K8sService {
    /// Service metadata.name (e.g., "productcatalogservice").
    pub name: String,
    /// Service namespace (from metadata.namespace, or "default").
    pub namespace: String,
    /// Ports exposed by the service (from spec.ports[].port).
    pub ports: Vec<u16>,
    /// Manifest file.
    pub file: String,
    /// Line number of the Service definition.
    pub line: u32,
    /// Selector labels (e.g., {"app": "productcatalogservice"}).
    pub selector: FxHashMap<String, String>,
}

/// A resolved match: code env var → K8s manifest env value → target service.
#[derive(Debug, Clone)]
pub struct K8sServiceMatch {
    /// The env var name from code (e.g., "PRODUCT_CATALOG_SERVICE_ADDR").
    pub env_var_name: String,
    /// The raw value from the K8s manifest.
    pub env_value: String,
    /// The target service name extracted from the value.
    pub target_service: String,
    /// The target port extracted from the value.
    pub target_port: Option<u16>,
    /// The deployment that defines this env var.
    pub source_deployment: String,
    /// File where the env var was read in code.
    pub code_file: String,
    /// Line where the env var was read in code.
    pub code_line: u32,
    /// K8s manifest file.
    pub k8s_file: String,
    /// K8s manifest line.
    pub k8s_line: u32,
    /// Confidence: 0.95 for exact env name match with parseable host:port.
    pub confidence: f32,
}

/// Parse K8s Deployment/StatefulSet/DaemonSet manifests for env var bindings.
///
/// Extracts `spec.containers[].env[].name + value` as `K8sEnvBinding`,
/// and returns container ports as part of `K8sService` (via `parse_k8s_services`).
pub fn parse_k8s_deployments(content: &str, file: &str) -> (Vec<K8sEnvBinding>, Vec<K8sService>) {
    let mut env_bindings = Vec::new();
    let mut services = Vec::new();

    // Split on `---` for multi-document YAML
    for doc in content.split("\n---") {
        let lines: Vec<&str> = doc.lines().collect();
        let kind = find_yaml_field(&lines, "kind");
        let metadata_name = find_metadata_name(&lines);

        match kind.as_deref() {
            Some("Deployment") | Some("StatefulSet") | Some("DaemonSet") => {
                let deployment_name = metadata_name.unwrap_or_default();
                if deployment_name.is_empty() {
                    continue;
                }

                // Extract env bindings
                let bindings = extract_env_bindings(&lines, file, &deployment_name);
                env_bindings.extend(bindings);

                // Extract container ports → create a "service-like" entry
                let ports = extract_container_ports(&lines);
                if !ports.is_empty() {
                    services.push(K8sService {
                        name: deployment_name.clone(),
                        namespace: find_metadata_namespace(&lines)
                            .unwrap_or_else(|| "default".to_string()),
                        ports,
                        file: file.to_string(),
                        line: find_line_of_field(&lines, "kind").unwrap_or(1),
                        selector: FxHashMap::default(),
                    });
                }
            }
            Some("Service") => {
                if let Some(svc) = parse_single_service(&lines, file) {
                    services.push(svc);
                }
            }
            _ => {}
        }
    }

    (env_bindings, services)
}

/// Parse only K8s Service documents from a YAML file.
pub fn parse_k8s_services(content: &str, file: &str) -> Vec<K8sService> {
    let mut services = Vec::new();

    for doc in content.split("\n---") {
        let lines: Vec<&str> = doc.lines().collect();
        let kind = find_yaml_field(&lines, "kind");
        if kind.as_deref() == Some("Service") {
            if let Some(svc) = parse_single_service(&lines, file) {
                services.push(svc);
            }
        }
    }

    services
}

/// Extract a service name and port from a K8s-style address value.
///
/// Parses patterns like:
/// - "productcatalogservice:3550" → ("productcatalogservice", 3550)
/// - "service.namespace.svc.cluster.local:8080" → ("service", 8080)
/// - "service.namespace:8080" → ("service", 8080)
/// - "service" → ("service", None) — no port
pub fn extract_service_from_value(value: &str) -> Option<(String, Option<u16>)> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return None;
    }

    // Strip scheme if present (http://, grpc://, etc.)
    let without_scheme = if let Some(idx) = trimmed.find("://") {
        &trimmed[idx + 3..]
    } else {
        trimmed
    };

    // Strip path if present
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);

    // Handle K8s full DNS: service.namespace.svc.cluster.local:port
    let parts: Vec<&str> = host_port.split('.').collect();

    // Extract the service name (first component before any dots)
    let (host, port) = if let Some(colon_idx) = host_port.rfind(':') {
        let port_str = &host_port[colon_idx + 1..];
        match port_str.parse::<u16>() {
            Ok(p) => (&host_port[..colon_idx], Some(p)),
            Err(_) => (host_port, None),
        }
    } else {
        (host_port, None)
    };

    // Service name is the first DNS component
    let service_name = if parts.len() > 1 {
        parts[0].to_string()
    } else if let Some(colon_idx) = host.find(':') {
        host[..colon_idx].to_string()
    } else {
        host.to_string()
    };

    // Filter out obviously non-service values
    if service_name.is_empty()
        || service_name.contains(' ')
        || service_name.starts_with("{{")
        || service_name.parse::<u16>().is_ok()
    {
        return None;
    }

    Some((service_name, port))
}

/// Match env var names read in code against K8s manifest env var values.
///
/// When code does `os.Getenv("PRODUCT_CATALOG_SERVICE_ADDR")` and K8s sets
/// `PRODUCT_CATALOG_SERVICE_ADDR=productcatalogservice:3550`, this links them.
pub fn match_env_to_services(
    env_bindings: &[K8sEnvBinding],
    code_env_vars: &[EnvVarRead],
) -> Vec<K8sServiceMatch> {
    let mut matches = Vec::new();

    // Index K8s env bindings by var name
    let mut k8s_index: FxHashMap<&str, Vec<&K8sEnvBinding>> = FxHashMap::default();
    for binding in env_bindings {
        k8s_index
            .entry(&binding.var_name)
            .or_default()
            .push(binding);
    }

    for code_read in code_env_vars {
        if let Some(bindings) = k8s_index.get(code_read.var_name.as_str()) {
            for binding in bindings {
                if let Some((service, port)) = extract_service_from_value(&binding.value) {
                    matches.push(K8sServiceMatch {
                        env_var_name: code_read.var_name.clone(),
                        env_value: binding.value.clone(),
                        target_service: service,
                        target_port: port,
                        source_deployment: binding.deployment_name.clone(),
                        code_file: code_read.file.clone(),
                        code_line: code_read.line,
                        k8s_file: binding.file.clone(),
                        k8s_line: binding.line,
                        confidence: if port.is_some() { 0.95 } else { 0.80 },
                    });
                }
            }
        }
    }

    matches
}

// ── Internal helpers ──────────────────────────────────────────────

/// Find a top-level YAML field value (e.g., "kind: Deployment" → "Deployment").
fn find_yaml_field(lines: &[&str], field: &str) -> Option<String> {
    let prefix = format!("{}:", field);
    for line in lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let val = rest.trim().trim_matches('"').trim_matches('\'');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Find the line number (1-indexed) of a YAML field.
fn find_line_of_field(lines: &[&str], field: &str) -> Option<u32> {
    let prefix = format!("{}:", field);
    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with(&prefix) {
            return Some((i + 1) as u32);
        }
    }
    None
}

/// Find `metadata.name` in a K8s manifest.
/// Looks for `name:` line that appears right after or under `metadata:`.
fn find_metadata_name(lines: &[&str]) -> Option<String> {
    let mut in_metadata = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "metadata:" {
            in_metadata = true;
            continue;
        }
        if in_metadata {
            if let Some(rest) = trimmed.strip_prefix("name:") {
                let val = rest.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
            // Stop if we've left the metadata block (non-indented line)
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_metadata = false;
            }
        }
    }
    None
}

/// Find `metadata.namespace` in a K8s manifest.
fn find_metadata_namespace(lines: &[&str]) -> Option<String> {
    let mut in_metadata = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "metadata:" {
            in_metadata = true;
            continue;
        }
        if in_metadata {
            if let Some(rest) = trimmed.strip_prefix("namespace:") {
                let val = rest.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
            if !line.starts_with(' ') && !line.starts_with('\t') && !trimmed.is_empty() {
                in_metadata = false;
            }
        }
    }
    None
}

/// Extract env var bindings from a Deployment spec.
fn extract_env_bindings(lines: &[&str], file: &str, deployment_name: &str) -> Vec<K8sEnvBinding> {
    let mut bindings = Vec::new();
    let mut pending_name: Option<String> = None;
    let mut pending_line: u32 = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;

        // Look for `- name: VAR_NAME` under env:
        if trimmed.starts_with("- name:") {
            if let Some(rest) = trimmed.strip_prefix("- name:") {
                let name = rest.trim().trim_matches('"').trim_matches('\'');
                if looks_like_env_var(name) {
                    pending_name = Some(name.to_string());
                    pending_line = line_num;
                } else {
                    pending_name = None;
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("value:") {
            if let Some(var_name) = pending_name.take() {
                let value = rest.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    bindings.push(K8sEnvBinding {
                        var_name,
                        value: value.to_string(),
                        file: file.to_string(),
                        line: pending_line,
                        deployment_name: deployment_name.to_string(),
                    });
                }
            }
        } else if trimmed.starts_with("valueFrom:") {
            // valueFrom: means it's a reference, not a literal — skip
            pending_name = None;
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("value")
            && !trimmed.starts_with('-')
        {
            // Reset if we've moved past the name/value pair
            // but only if it's not an indented continuation
            if pending_name.is_some()
                && !line.starts_with("          ")
                && !line.starts_with("\t\t\t")
            {
                pending_name = None;
            }
        }
    }

    bindings
}

/// Extract containerPort values from containers spec.
fn extract_container_ports(lines: &[&str]) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("containerPort:") {
            if let Ok(port) = rest.trim().parse::<u16>() {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
    }
    ports
}

/// Parse a single K8s Service document.
fn parse_single_service(lines: &[&str], file: &str) -> Option<K8sService> {
    let name = find_metadata_name(lines)?;
    let namespace = find_metadata_namespace(lines).unwrap_or_else(|| "default".to_string());
    let line = find_line_of_field(lines, "kind").unwrap_or(1);

    // Extract spec.ports[].port
    let mut ports = Vec::new();
    let mut in_ports = false;
    for l in lines {
        let trimmed = l.trim();
        if trimmed == "ports:" {
            in_ports = true;
            continue;
        }
        if in_ports {
            if let Some(rest) = trimmed.strip_prefix("port:") {
                if let Ok(p) = rest.trim().parse::<u16>() {
                    ports.push(p);
                }
            } else if let Some(rest) = trimmed.strip_prefix("- port:") {
                if let Ok(p) = rest.trim().parse::<u16>() {
                    ports.push(p);
                }
            }
            // Stop when we leave the ports section
            if !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with("port:")
                && !trimmed.starts_with("targetPort:")
                && !trimmed.starts_with("protocol:")
                && !trimmed.starts_with("name:")
                && !trimmed.starts_with("nodePort:")
            {
                in_ports = false;
            }
        }
    }

    // Extract spec.selector
    let mut selector = FxHashMap::default();
    let mut in_selector = false;
    for l in lines {
        let trimmed = l.trim();
        if trimmed == "selector:" {
            in_selector = true;
            continue;
        }
        if in_selector {
            if let Some(colon_pos) = trimmed.find(':') {
                let key = trimmed[..colon_pos].trim();
                let val = trimmed[colon_pos + 1..].trim().trim_matches('"');
                if !key.is_empty() && !val.is_empty() && !key.starts_with('-') {
                    // Stop if we hit a section that's not a selector label
                    if key == "ports" || key == "type" || key == "clusterIP" {
                        in_selector = false;
                    } else {
                        selector.insert(key.to_string(), val.to_string());
                    }
                }
            }
            if !l.starts_with(' ') && !l.starts_with('\t') && !trimmed.is_empty() {
                in_selector = false;
            }
        }
    }

    Some(K8sService {
        name,
        namespace,
        ports,
        file: file.to_string(),
        line,
        selector,
    })
}

/// Check if a string looks like an env var name (mostly uppercase + underscores).
fn looks_like_env_var(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let alpha_count = name.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if alpha_count == 0 {
        return false;
    }
    let upper_count = name.chars().filter(|c| c.is_ascii_uppercase()).count();
    upper_count * 100 / alpha_count >= 60
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONTEND_YAML: &str = r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: frontend
spec:
  selector:
    matchLabels:
      app: frontend
  template:
    spec:
      containers:
        - name: server
          image: frontend
          ports:
          - containerPort: 8080
          env:
          - name: PORT
            value: "8080"
          - name: PRODUCT_CATALOG_SERVICE_ADDR
            value: "productcatalogservice:3550"
          - name: CURRENCY_SERVICE_ADDR
            value: "currencyservice:7000"
          - name: CART_SERVICE_ADDR
            value: "cartservice:7070"
          - name: RECOMMENDATION_SERVICE_ADDR
            value: "recommendationservice:8080"
          - name: SHIPPING_SERVICE_ADDR
            value: "shippingservice:50051"
          - name: CHECKOUT_SERVICE_ADDR
            value: "checkoutservice:5050"
          - name: AD_SERVICE_ADDR
            value: "adservice:9555"
---
apiVersion: v1
kind: Service
metadata:
  name: frontend
spec:
  type: ClusterIP
  selector:
    app: frontend
  ports:
  - name: http
    port: 80
    targetPort: 8080"#;

    const PRODUCTCATALOG_YAML: &str = r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: productcatalogservice
spec:
  selector:
    matchLabels:
      app: productcatalogservice
  template:
    spec:
      containers:
      - name: server
        image: productcatalogservice
        ports:
        - containerPort: 3550
        env:
        - name: PORT
          value: "3550"
---
apiVersion: v1
kind: Service
metadata:
  name: productcatalogservice
spec:
  type: ClusterIP
  selector:
    app: productcatalogservice
  ports:
  - name: grpc
    port: 3550
    targetPort: 3550"#;

    #[test]
    fn parse_frontend_deployment_env_bindings() {
        let (bindings, _) = parse_k8s_deployments(FRONTEND_YAML, "frontend.yaml");
        assert!(bindings.len() >= 7, "got {} bindings", bindings.len());

        let product = bindings
            .iter()
            .find(|b| b.var_name == "PRODUCT_CATALOG_SERVICE_ADDR")
            .expect("should find PRODUCT_CATALOG_SERVICE_ADDR");
        assert_eq!(product.value, "productcatalogservice:3550");
        assert_eq!(product.deployment_name, "frontend");
        assert_eq!(product.file, "frontend.yaml");

        let cart = bindings
            .iter()
            .find(|b| b.var_name == "CART_SERVICE_ADDR")
            .expect("should find CART_SERVICE_ADDR");
        assert_eq!(cart.value, "cartservice:7070");
    }

    #[test]
    fn parse_frontend_services() {
        let (_, services) = parse_k8s_deployments(FRONTEND_YAML, "frontend.yaml");
        let svc = services
            .iter()
            .find(|s| s.name == "frontend" && s.ports.contains(&80))
            .expect("should find frontend Service with port 80");
        assert_eq!(svc.namespace, "default");
    }

    #[test]
    fn parse_productcatalog_deployment() {
        let (bindings, services) =
            parse_k8s_deployments(PRODUCTCATALOG_YAML, "productcatalogservice.yaml");
        assert!(bindings.iter().any(|b| b.var_name == "PORT" && b.value == "3550"));

        let svc = services
            .iter()
            .find(|s| s.name == "productcatalogservice" && s.ports.contains(&3550))
            .expect("should find productcatalogservice with port 3550");
        assert_eq!(svc.namespace, "default");
    }

    #[test]
    fn extract_service_host_port() {
        let (svc, port) =
            extract_service_from_value("productcatalogservice:3550").unwrap();
        assert_eq!(svc, "productcatalogservice");
        assert_eq!(port, Some(3550));
    }

    #[test]
    fn extract_service_full_dns() {
        let (svc, port) = extract_service_from_value(
            "productcatalog.default.svc.cluster.local:3550",
        )
        .unwrap();
        assert_eq!(svc, "productcatalog");
        assert_eq!(port, Some(3550));
    }

    #[test]
    fn extract_service_no_port() {
        let (svc, port) = extract_service_from_value("cartservice").unwrap();
        assert_eq!(svc, "cartservice");
        assert_eq!(port, None);
    }

    #[test]
    fn extract_service_with_scheme() {
        let (svc, port) =
            extract_service_from_value("http://myservice:8080/api").unwrap();
        assert_eq!(svc, "myservice");
        assert_eq!(port, Some(8080));
    }

    #[test]
    fn extract_service_empty_returns_none() {
        assert!(extract_service_from_value("").is_none());
    }

    #[test]
    fn extract_service_template_returns_none() {
        assert!(extract_service_from_value("{{ .Values.addr }}").is_none());
    }

    #[test]
    fn match_env_to_k8s_services() {
        let (bindings, _) = parse_k8s_deployments(FRONTEND_YAML, "frontend.yaml");

        let code_env_vars = vec![
            EnvVarRead {
                var_name: "PRODUCT_CATALOG_SERVICE_ADDR".into(),
                file: "main.go".into(),
                line: 42,
            },
            EnvVarRead {
                var_name: "CART_SERVICE_ADDR".into(),
                file: "main.go".into(),
                line: 50,
            },
            EnvVarRead {
                var_name: "NONEXISTENT_VAR".into(),
                file: "main.go".into(),
                line: 99,
            },
        ];

        let matches = match_env_to_services(&bindings, &code_env_vars);
        assert_eq!(matches.len(), 2, "should match 2 out of 3 env vars");

        let product_match = matches
            .iter()
            .find(|m| m.env_var_name == "PRODUCT_CATALOG_SERVICE_ADDR")
            .expect("should match PRODUCT_CATALOG_SERVICE_ADDR");
        assert_eq!(product_match.target_service, "productcatalogservice");
        assert_eq!(product_match.target_port, Some(3550));
        assert_eq!(product_match.source_deployment, "frontend");
        assert!(product_match.confidence >= 0.90);

        let cart_match = matches
            .iter()
            .find(|m| m.env_var_name == "CART_SERVICE_ADDR")
            .expect("should match CART_SERVICE_ADDR");
        assert_eq!(cart_match.target_service, "cartservice");
        assert_eq!(cart_match.target_port, Some(7070));
    }

    #[test]
    fn parse_k8s_services_only() {
        let services = parse_k8s_services(PRODUCTCATALOG_YAML, "productcatalogservice.yaml");
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "productcatalogservice");
        assert!(services[0].ports.contains(&3550));
    }

    #[test]
    fn test_with_real_fixture() {
        let fixture =
            std::fs::read_to_string("/tmp/cx-test-repos/microservices-demo/kubernetes-manifests/frontend.yaml");
        if let Ok(content) = fixture {
            let (bindings, services) = parse_k8s_deployments(&content, "frontend.yaml");

            let product = bindings
                .iter()
                .find(|b| b.var_name == "PRODUCT_CATALOG_SERVICE_ADDR");
            assert!(
                product.is_some(),
                "should find PRODUCT_CATALOG_SERVICE_ADDR in real fixture"
            );
            assert_eq!(product.unwrap().value, "productcatalogservice:3550");

            // Should have both the Deployment ports and at least one Service
            assert!(
                services.iter().any(|s| s.name == "frontend"),
                "should find frontend service"
            );
        }
    }

    #[test]
    fn test_full_chain_product_catalog_resolution() {
        // Read all K8s manifests from the test fixture
        let manifests_dir =
            "/tmp/cx-test-repos/microservices-demo/kubernetes-manifests";
        let mut all_bindings = Vec::new();
        let mut all_services = Vec::new();

        if let Ok(entries) = std::fs::read_dir(manifests_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                        let (bindings, services) =
                            parse_k8s_deployments(&content, &file_name);
                        all_bindings.extend(bindings);
                        all_services.extend(services);
                    }
                }
            }
        }

        if all_bindings.is_empty() {
            return; // fixture not available
        }

        // Simulate code reading PRODUCT_CATALOG_SERVICE_ADDR
        let code_env_vars = vec![EnvVarRead {
            var_name: "PRODUCT_CATALOG_SERVICE_ADDR".into(),
            file: "main.go".into(),
            line: 42,
        }];

        let matches = match_env_to_services(&all_bindings, &code_env_vars);
        assert!(
            !matches.is_empty(),
            "PRODUCT_CATALOG_SERVICE_ADDR should resolve"
        );

        let m = &matches[0];
        assert_eq!(m.target_service, "productcatalogservice");
        assert_eq!(m.target_port, Some(3550));

        // Verify the target service exists in our parsed services
        assert!(
            all_services
                .iter()
                .any(|s| s.name == "productcatalogservice"),
            "productcatalogservice should be in parsed services"
        );
    }
}
