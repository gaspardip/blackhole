const host = navigator; const credentials = host.credentials; Object.defineProperty(credentials, "get", { value: get });
