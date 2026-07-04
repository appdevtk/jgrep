select(.kind == "Deployment" and .spec.replicas > 2) | .metadata.name
