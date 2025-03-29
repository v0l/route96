# Deployment
Verse utilizes a combination of Helm and ArgoCD to deploy this application to its Kubernetes cluster. Configuration for this deployment pattern requires the following:
    1. The repository has a helm chart, and all required/desired templates, written and stored in its `/deployment/<chart>` directory.
    2. An ArgoCD Application has been created that targets this repository's `/deployment/<chart>` directory.

## To Deploy a change
    1. Update the `/deployment/Chart.yaml`'s `AppVersion` file, to contain the tag for the new Docker image.
    2. Merge this update to main.
    3. In ArgoCD, if auto-sync is not enabled for the Application that was created (the one targeting this repo), execute the sync operation for this Application.
    4. You should then see the new version of the application replace the old one, in ArgoCD. This reflects what is happening in the cluster.