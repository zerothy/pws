use std::{collections::HashMap, process::Stdio};

use anyhow::Result;
use serde_json;
use bollard::network::DisconnectNetworkOptions;
use bollard::{
    container::{Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions},
    image::{BuildImageOptions, ListImagesOptions, TagImageOptions},
    network::{ConnectNetworkOptions, InspectNetworkOptions, ListNetworksOptions},
    service::{HostConfig, NetworkContainer, RestartPolicy, RestartPolicyNameEnum},
    Docker,
};
use futures_util::stream::TryStreamExt;
use hyper::Body;
use tar::Builder;
use std::io::Cursor;
use crate::dockerfile_templates::DjangoDockerfile;
use sqlx::PgPool;
use tokio::process::Command;

use crate::get_env;

pub struct DockerContainer {
    pub ip: String,
    pub port: i32,
    pub build_log: String,
}

#[tracing::instrument(skip(pool))]
pub async fn build_docker(
    owner: &str,
    project_name: &str,
    container_name: &str,
    container_src: &str,
    pool: PgPool,
) -> Result<DockerContainer> {
    let image_name = format!("{}:latest", container_name);
    let old_image_name = format!("{}:old", container_name);
    let network_name = "pemasak".to_string(); // Use shared network for Traefik

    let docker = Docker::connect_with_local_defaults().map_err(|err| {
        tracing::error!("Failed to connect to docker: {}", err);
        err
    })?;

    // check if image exists
    let images = &docker
        .list_images(Some(ListImagesOptions::<String> {
            all: false,
            filters: HashMap::from([("reference".to_string(), vec![image_name.to_string()])]),
            ..Default::default()
        }))
        .await
        .map_err(|err| {
            tracing::error!("Failed to list images: {}", err);
            err
        })?;

    // remove image if it exists
    if let Some(_image) = images.first() {
        let tag_options = TagImageOptions {
            tag: "old",
            repo: container_name,
        };

        docker
            .tag_image(container_name, Some(tag_options))
            .await
            .map_err(|err| {
                tracing::error!("Failed to tag image: {}", err);
                err
            })?;

        docker
            .remove_image(&image_name, None, None)
            .await
            .map_err(|err| {
                tracing::error!("Failed to remove image: {}", err);
                err
            })?;
    };

    // Get user environment variables for Django
    let envs = sqlx::query!(
        r#"SELECT environs 
        FROM projects
        JOIN project_owners ON projects.owner_id = project_owners.id
        WHERE projects.name = $1 AND project_owners.name = $2"#,
        project_name, owner,
    )
    .fetch_one(&pool)
    .await
    .map_err(|err| {
        tracing::error!("Failed to query database: {}", err);
        err
    })?;

    tracing::info!("BUILDING START");

    let build_log = match std::path::Path::new(container_src)
        .join("Dockerfile")
        .exists()
    {
        true => {
            tracing::debug!(container_name, "Build using existing dockerfile");
            // build from existing Dockerfile with user env vars as build args
            let mut cmd = Command::new("docker");
            let mut args = vec![
                "build".to_string(),
                "--cpu-period=100000".to_string(),
                "--cpu-quota=50000".to_string(),
                "-t".to_string(),
                image_name.clone(),
                "-f".to_string(),
                std::path::Path::new(container_src)
                    .join("Dockerfile")
                    .to_str()
                    .unwrap()
                    .to_string(),
            ];
            
            // Add environment variables as build args
            if let Some(env_map) = envs.environs.as_object() {
                for (key, value) in env_map {
                    args.push("--build-arg".to_string());
                    args.push(format!("{}={}", key, value.as_str().unwrap_or("")));
                }
                tracing::debug!(container_name, "Added {} build args", env_map.len());
            }
            
            args.push(container_src.to_string());
            cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

            let child = cmd.spawn().map_err(|err| {
                tracing::error!("Failed to spawn docker build: {}", err);
                err
            })?;

            let output = child.wait_with_output().await.map_err(|err| {
                tracing::error!("Failed to wait for docker build: {}", err);
                err
            })?;

            if !output.status.success() {
                return Err(anyhow::anyhow!(String::from_utf8(output.stderr).unwrap()));
            }
            String::from_utf8(output.stderr).unwrap()
        }
        false => {
            tracing::debug!(container_name, "Generating efficient Django Dockerfile");
            
            // Generate our efficient multi-stage Dockerfile with environment variables
            let environment_strings = match envs.environs.as_object() {
                Some(map) => {
                    map.into_iter().map(|(key, value)| {
                        format!("{}={}", key, value.as_str().unwrap_or(""))
                    }).collect::<Vec<_>>()
                },
                None => Vec::new(),
            };
            
            let django_dockerfile = DjangoDockerfile::new().with_environment(environment_strings);
            let dockerfile_content = django_dockerfile.generate();
            
            tracing::info!("Generated efficient Django Dockerfile in-memory");
            
            // Create in-memory tar archive with Dockerfile content
            let mut tar_data = Vec::new();
            {
                let mut tar_builder = Builder::new(&mut tar_data);
                
                // Add Dockerfile to tar
                let mut dockerfile_header = tar::Header::new_gnu();
                dockerfile_header.set_path("Dockerfile").map_err(|err| {
                    tracing::error!("Failed to set Dockerfile path: {}", err);
                    anyhow::anyhow!("Failed to set Dockerfile path: {}", err)
                })?;
                dockerfile_header.set_size(dockerfile_content.len() as u64);
                dockerfile_header.set_mode(0o644);
                dockerfile_header.set_cksum();
                
                tar_builder.append(&dockerfile_header, dockerfile_content.as_bytes()).map_err(|err| {
                    tracing::error!("Failed to append Dockerfile to tar: {}", err);
                    anyhow::anyhow!("Failed to append Dockerfile to tar: {}", err)
                })?;
                
                // Add source files to tar  
                tar_builder.append_dir_all(".", container_src).map_err(|err| {
                    tracing::error!("Failed to append source directory to tar: {}", err);
                    anyhow::anyhow!("Failed to append source directory to tar: {}", err)
                })?;
                
                tar_builder.finish().map_err(|err| {
                    tracing::error!("Failed to finish tar archive: {}", err);
                    anyhow::anyhow!("Failed to finish tar archive: {}", err)
                })?;
            }
            
            // Build using Bollard API with in-memory tar stream
            let build_options = BuildImageOptions {
                dockerfile: "Dockerfile".to_string(),
                t: image_name.clone(),
                rm: true,
                pull: true,
                cpuperiod: Some(100000),
                cpuquota: Some(50000),
                ..Default::default()
            };
            
            // Convert tar data to hyper::Body
            let tar_body = Body::from(bytes::Bytes::from(tar_data));
            
            let mut build_result = docker.build_image(build_options, None, Some(tar_body));
            let mut build_log = String::new();
            
            while let Some(build_info) = build_result.try_next().await.map_err(|err| {
                tracing::error!("Docker build failed: {}", err);
                anyhow::anyhow!("Docker build failed: {}", err)
            })? {
                if let Some(stream) = build_info.stream {
                    build_log.push_str(&stream);
                    tracing::debug!("Build: {}", stream.trim());
                }
                if let Some(error) = build_info.error {
                    tracing::error!("Build error: {}", error);
                    return Err(anyhow::anyhow!("Docker build failed: {}", error));
                }
            }
            
            build_log
        }
    };

    // check if image exists
    let images = &docker
        .list_images(Some(ListImagesOptions::<String> {
            all: false,
            filters: HashMap::from([("reference".to_string(), vec![image_name.to_string()])]),
            ..Default::default()
        }))
        .await
        .map_err(|err| {
            tracing::error!("Failed to list images: {}", err);
            err
        })?;

    let _image = images.first().ok_or(anyhow::anyhow!("No image found"))?;

    // check if container exists
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            filters: HashMap::from([("name".to_string(), vec![format!("^{container_name}$")])]),
            ..Default::default()
        }))
        .await
        .map_err(|err| {
            tracing::error!("Failed to list containers: {}", err);
            err
        })?
        .into_iter()
        .collect::<Vec<_>>();

    // remove container if it exists
    if !containers.is_empty() {
        docker
            .stop_container(container_name, None)
            .await
            .map_err(|err| {
                tracing::error!("Failed to stop container: {}", err);
                err
            })?;

        docker
            .remove_container(containers.first().unwrap().id.as_ref().unwrap(), None)
            .await
            .map_err(|err| {
                tracing::error!("Failed to remove container: {}", err);
                err
            })?;

        docker
            .remove_image(&old_image_name, None, None)
            .await
            .map_err(|err| {
                tracing::error!("Failed to remove image: {}", err);
                err
            })?;
    }

    // check if network exists
    let network = docker
        .list_networks(Some(ListNetworksOptions {
            filters: HashMap::from([("name".to_string(), vec![network_name.to_string()])]),
        }))
        .await
        .map_err(|err| {
            tracing::error!("Failed to list networks: {}", err);
            err
        })?
        .first()
        .map(|n| n.to_owned());

    // create network if it doesn't exist
    let network = match network {
        Some(n) => {
            tracing::info!("Existing network id -> {:?}", n.id);
            n
        }
        None => {
            let options = bollard::network::CreateNetworkOptions {
                name: network_name.clone(),
                ..Default::default()
            };
            let res = docker.create_network(options).await.map_err(|err| {
                tracing::error!("Failed to create network: {}", err);
                err
            })?;
            tracing::info!("create network response-> {:#?}", res);

            docker
                .list_networks(Some(ListNetworksOptions {
                    filters: HashMap::from([("name".to_string(), vec![network_name.to_string()])]),
                }))
                .await?
                .first()
                .map(|n| n.to_owned())
                .ok_or(anyhow::anyhow!("No network found after make one???"))?
        }
    };

    // TODO: figure out if we need make this configurable
    let port = 80;

    let envs = sqlx::query!(
        r#"SELECT environs 
        FROM projects
        JOIN project_owners ON projects.owner_id = project_owners.id
        WHERE projects.name = $1 AND project_owners.name = $2"#,
        project_name, owner,
    )
    .fetch_one(&pool)
    .await
    .map_err(|err| {
        tracing::error!(?err, "Failed to query database: {}", err);
        err
    })?;

    let environment_strings = match envs.environs.as_object() {
        Some(map) => {
            let environment_strings = map.into_iter().map(|(key, value)| {
                format!("{}={}", key, value.as_str().unwrap())
            }).collect::<Vec<_>>();

            Ok(environment_strings)
        },
        None => {
            tracing::error!("Non object value passed as environment variable {}", container_name);
            Err(anyhow::anyhow!("Non object value passed as environment variable {}", container_name))
        }
    }?;


    let config: Config<String> = Config {
        image: Some(image_name.clone()),
        env: Some(environment_strings),
        // Auto-add Traefik labels for PWS deployed containers with HTTPS
        labels: Some(HashMap::from([
            ("traefik.enable".to_string(), "true".to_string()),
            (format!("traefik.http.routers.{}.rule", container_name), format!("Host(`{}.{}`)", container_name, get_env::domain())),
            (format!("traefik.http.routers.{}.entrypoints", container_name), "websecure".to_string()),
            (format!("traefik.http.routers.{}.tls.certresolver", container_name), "letsencrypt".to_string()),
            (format!("traefik.http.services.{}.loadbalancer.server.port", container_name), "80".to_string()),
        ])),
        host_config: Some(HostConfig {
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::ON_FAILURE),
                ..Default::default()
            }),
            // Resource limits for 200+ students - prevent resource abuse
            memory: Some(256 * 1024 * 1024),        // 256MB memory limit
            memory_swap: Some(320 * 1024 * 1024),   // 320MB total (256M + 64M swap)
            cpu_quota: Some(50000),                  // 0.5 CPU (50% of 100000 period)
            cpu_period: Some(100000),                // Standard 100ms period
            ..Default::default()
        }),
        ..Default::default()
    };

    let res = docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name,
                platform: None,
            }),
            config,
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to create container: {}", err);
            err
        })?;

    tracing::info!("create response-> {:#?}", res);

    // connect container to network
    docker
        .connect_network(
            &network_name,
            ConnectNetworkOptions {
                container: container_name,
                ..Default::default()
            },
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to connect network: {}", err);
            err
        })?;

    docker
        .start_container(container_name, None::<StartContainerOptions<&str>>)
        .await
        .map_err(|err| {
            tracing::error!("Failed to start container: {}", err);
            err
        })?;

    //inspect network
    let network_inspect = docker
        .inspect_network(
            &network.id.unwrap(),
            Some(InspectNetworkOptions::<&str> {
                verbose: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to inspect network: {}", err);
            err
        })?;

    let network_container = network_inspect
        .containers
        .unwrap_or_default()
        .get(&res.id)
        .unwrap()
        .clone();

    // TODO: this network if for one block. We need to makesure that we can get the right ip
    // attached to the container
    let NetworkContainer {
        ipv4_address,
        ipv6_address,
        ..
    } = network_container;

    tracing::info!(ipv4_address = ?ipv4_address, ipv6_address = ?ipv6_address, "Container {} ip addresses", container_name);

    // TODO: make this configurable
    let ip = ipv6_address
        .filter(|ip| !ip.is_empty())
        .or(ipv4_address.filter(|ip| !ip.is_empty()))
        .and_then(|ip| ip.split('/').next().map(|ip| ip.to_string()))
        .ok_or_else(|| {
            tracing::error!("No ip address found for container {}", container_name);
            anyhow::anyhow!("No ip address found for container {}", container_name)
        })?;

    tracing::info!(ip = ?ip, port = ?port, "Container {} ip address", container_name);

    let _ = docker
        .disconnect_network(
            "bridge",
            DisconnectNetworkOptions {
                container: container_name,
                force: true,
            },
        )
        .await
        .map_err(|err| {
            tracing::error!("Failed to disconnect container from bridge: {}", err);
            err
        });

    Ok(DockerContainer {
        ip,
        port,
        build_log,
    })
}