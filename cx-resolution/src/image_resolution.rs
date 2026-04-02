use rustc_hash::FxHashMap;

/// A Docker image built by a Dockerfile.
#[derive(Debug, Clone)]
pub struct DockerImage {
    /// The full image reference (e.g., "gcr.io/example-org/myapp/tts-server").
    pub image_ref: String,
    /// Dockerfile path.
    pub file: String,
}

/// A container image reference from a k8s deployment/values file.
#[derive(Debug, Clone)]
pub struct K8sContainerImage {
    /// The full image reference (e.g., "gcr.io/example-org/myapp/tts-server:v1.2.3").
    pub image_ref: String,
    /// The k8s manifest or Helm values file.
    pub file: String,
    /// Line number.
    pub line: u32,
    /// The deployment/pod name if known.
    pub deployment_name: Option<String>,
}

/// A resolved image match: Dockerfile → k8s deployment.
#[derive(Debug, Clone)]
pub struct ImageMatch {
    /// The matched image path (without tag).
    pub image_path: String,
    /// Dockerfile info.
    pub dockerfile: String,
    pub dockerfile_repo: String,
    /// K8s deployment info.
    pub k8s_file: String,
    pub k8s_line: u32,
    pub k8s_repo: String,
    pub deployment_name: Option<String>,
    /// Confidence score.
    pub confidence: f32,
}

/// Strip the tag from a Docker image reference.
/// "gcr.io/org/app:v1.2.3" → "gcr.io/org/app"
/// "gcr.io/org/app@sha256:..." → "gcr.io/org/app"
fn strip_tag(image_ref: &str) -> &str {
    let s = image_ref.trim();
    if let Some(i) = s.find('@') {
        return &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        // Make sure the colon is after the last slash (not a port in registry)
        if let Some(slash_i) = s.rfind('/') {
            if i > slash_i {
                return &s[..i];
            }
        } else {
            return &s[..i];
        }
    }
    s
}

/// Extract just the image name (last path component) from a reference.
/// "gcr.io/example-org/myapp/tts-server" → "tts-streaming-server"
fn image_name(image_ref: &str) -> &str {
    let stripped = strip_tag(image_ref);
    stripped.rsplit('/').next().unwrap_or(stripped)
}

/// Match Docker images to k8s container image references.
pub fn match_images(
    docker_images: &[(String, Vec<DockerImage>)],
    k8s_images: &[(String, Vec<K8sContainerImage>)],
) -> Vec<ImageMatch> {
    let mut matches = Vec::new();

    // Build index by stripped image path for exact match
    let mut docker_by_path: FxHashMap<&str, Vec<(&str, &DockerImage)>> = FxHashMap::default();
    // Build index by image name for name-only match
    let mut docker_by_name: FxHashMap<&str, Vec<(&str, &DockerImage)>> = FxHashMap::default();

    for (repo, images) in docker_images {
        for img in images {
            let path = strip_tag(&img.image_ref);
            docker_by_path.entry(path).or_default().push((repo, img));
            let name = image_name(&img.image_ref);
            docker_by_name.entry(name).or_default().push((repo, img));
        }
    }

    for (k8s_repo, images) in k8s_images {
        for k8s_img in images {
            let k8s_path = strip_tag(&k8s_img.image_ref);
            let k8s_name = image_name(&k8s_img.image_ref);

            // Try exact path match first
            if let Some(dockers) = docker_by_path.get(k8s_path) {
                for &(docker_repo, docker_img) in dockers {
                    matches.push(ImageMatch {
                        image_path: k8s_path.to_string(),
                        dockerfile: docker_img.file.clone(),
                        dockerfile_repo: docker_repo.to_string(),
                        k8s_file: k8s_img.file.clone(),
                        k8s_line: k8s_img.line,
                        k8s_repo: k8s_repo.clone(),
                        deployment_name: k8s_img.deployment_name.clone(),
                        confidence: 0.95,
                    });
                }
                continue;
            }

            // Try name-only match
            if let Some(dockers) = docker_by_name.get(k8s_name) {
                for &(docker_repo, docker_img) in dockers {
                    matches.push(ImageMatch {
                        image_path: k8s_name.to_string(),
                        dockerfile: docker_img.file.clone(),
                        dockerfile_repo: docker_repo.to_string(),
                        k8s_file: k8s_img.file.clone(),
                        k8s_line: k8s_img.line,
                        k8s_repo: k8s_repo.clone(),
                        deployment_name: k8s_img.deployment_name.clone(),
                        confidence: 0.7,
                    });
                }
            }
        }
    }

    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tag_with_version() {
        assert_eq!(strip_tag("gcr.io/org/app:v1.2.3"), "gcr.io/org/app");
    }

    #[test]
    fn strip_tag_with_sha() {
        assert_eq!(
            strip_tag("gcr.io/org/app@sha256:abc123"),
            "gcr.io/org/app"
        );
    }

    #[test]
    fn strip_tag_no_tag() {
        assert_eq!(strip_tag("gcr.io/org/app"), "gcr.io/org/app");
    }

    #[test]
    fn image_name_extraction() {
        assert_eq!(
            image_name("gcr.io/example-org/myapp/tts-server:v1"),
            "tts-server"
        );
        assert_eq!(image_name("nginx:latest"), "nginx");
    }

    #[test]
    fn exact_image_path_match() {
        let docker = vec![(
            "backend-service".into(),
            vec![DockerImage {
                image_ref: "gcr.io/example-org/myapp/tts-server".into(),
                file: "Dockerfile".into(),
            }],
        )];
        let k8s = vec![(
            "infra-k8s-config".into(),
            vec![K8sContainerImage {
                image_ref: "gcr.io/example-org/myapp/tts-server:v2.0".into(),
                file: "values.yaml".into(),
                line: 15,
                deployment_name: Some("tts-server".into()),
            }],
        )];

        let result = match_images(&docker, &k8s);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence >= 0.95);
        assert_eq!(result[0].dockerfile_repo, "backend-service");
        assert_eq!(result[0].k8s_repo, "infra-k8s-config");
    }

    #[test]
    fn name_only_match_lower_confidence() {
        let docker = vec![(
            "backend-service".into(),
            vec![DockerImage {
                image_ref: "gcr.io/example-org/myapp/tts-server".into(),
                file: "Dockerfile".into(),
            }],
        )];
        let k8s = vec![(
            "k8s-config".into(),
            vec![K8sContainerImage {
                image_ref: "us-docker.pkg.dev/other-project/tts-server:latest".into(),
                file: "deployment.yaml".into(),
                line: 20,
                deployment_name: None,
            }],
        )];

        let result = match_images(&docker, &k8s);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence >= 0.7);
        assert!(result[0].confidence < 0.95);
    }

    #[test]
    fn no_match() {
        let docker = vec![(
            "repo-a".into(),
            vec![DockerImage {
                image_ref: "gcr.io/org/app-a".into(),
                file: "Dockerfile".into(),
            }],
        )];
        let k8s = vec![(
            "repo-b".into(),
            vec![K8sContainerImage {
                image_ref: "gcr.io/org/app-b:latest".into(),
                file: "deploy.yaml".into(),
                line: 5,
                deployment_name: None,
            }],
        )];

        let result = match_images(&docker, &k8s);
        assert!(result.is_empty());
    }
}
