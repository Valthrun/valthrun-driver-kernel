$Headers = @{
    "Accept"               = "application/vnd.github+json"
    "Authorization"        = "Bearer $env:UEFI_MAPPER_GITHUB_TOKEN"
    "X-GitHub-Api-Version" = "2022-11-28"
}
$Payload = @{
    "event_type"     = "driver_updated"
    "client_payload" = @{
        "driver_authorization" = "$env:DRIVER_GITHUB_TOKEN"
        "driver_url"           = "https://api.github.com/repositories/673504681/actions/artifacts/$env:DRIVER_ARTIFACT_ID/zip"
        "driver_version"       = "$($env:GITHUB_SHA.Substring(0, 7))"
    }
}
Invoke-WebRequest -Uri "https://api.github.com/repos/Valthrun/valthrun-uefi-mapper/dispatches" `
    -Method POST `
    -Headers $Headers `
    -Body $(ConvertTo-Json $Payload)
    