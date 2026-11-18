fn global(property: &str) -> bool {
    matches!(
        property,
        "AbortController"
            | "AbortSignal"
            | "fetch"
            | "isTauri"
            | "__TAURI_INTERNALS__"
            | "document"
            | "CSS"
            | "scrollTo"
            | "scrollBy"
            | "visualViewport"
            | "addEventListener"
            | "removeEventListener"
            | "dispatchEvent"
            | "CompressionStream"
            | "DecompressionStream"
            | "Worker"
            | "SharedWorker"
            | "WebSocket"
            | "XMLHttpRequest"
            | "ResizeObserver"
            | "IntersectionObserver"
            | "MutationObserver"
            | "requestIdleCallback"
            | "cancelIdleCallback"
            | "requestAnimationFrame"
            | "cancelAnimationFrame"
            | "localStorage"
            | "sessionStorage"
            | "navigator"
            | "Notification"
            | "matchMedia"
            | "open"
            | "performance"
            | "screen"
            | "crypto"
            | "Date"
            | "FileReader"
            | "BroadcastChannel"
            | "innerWidth"
            | "innerHeight"
            | "devicePixelRatio"
    )
}
pub(super) fn protected(owner: &str, property: &str) -> bool {
    match owner {
        "" => global(property),
        "document" | "Document.prototype" => matches!(
            property,
            "querySelector"
                | "querySelectorAll"
                | "getElementById"
                | "createElement"
                | "addEventListener"
                | "removeEventListener"
                | "dispatchEvent"
                | "visibilityState"
                | "hidden"
                | "hasFocus"
                | "modelContext"
        ),
        "navigator" | "Navigator.prototype" => matches!(
            property,
            "clipboard"
                | "permissions"
                | "storage"
                | "serviceWorker"
                | "gpu"
                | "credentials"
                | "userAgent"
                | "userAgentData"
                | "webdriver"
                | "platform"
                | "onLine"
                | "modelContext"
        ),
        "FileReader.prototype" => matches!(
            property,
            "abort" | "readAsArrayBuffer" | "readAsBinaryString" | "readAsDataURL" | "readAsText"
        ),
        "IntersectionObserver.prototype" => matches!(
            property,
            "observe" | "unobserve" | "disconnect" | "takeRecords"
        ),
        "ResizeObserver.prototype" => matches!(property, "observe" | "unobserve" | "disconnect"),
        "MutationObserver.prototype" => {
            matches!(property, "observe" | "disconnect" | "takeRecords")
        }
        "AbortController.prototype" => matches!(property, "abort" | "signal"),
        "AbortSignal" => matches!(property, "abort" | "any" | "timeout"),
        "AbortSignal.prototype" => matches!(property, "aborted" | "reason" | "throwIfAborted"),
        "Date" | "performance" => matches!(property, "now"),
        "Math" => matches!(property, "random"),
        "crypto" => matches!(property, "randomUUID" | "getRandomValues"),
        "history" => matches!(
            property,
            "pushState" | "replaceState" | "go" | "back" | "forward"
        ),
        "Storage.prototype" | "localStorage" | "sessionStorage" => matches!(
            property,
            "getItem" | "setItem" | "removeItem" | "clear" | "key" | "length"
        ),
        "navigator.clipboard" => matches!(property, "read" | "readText" | "write" | "writeText"),
        "navigator.credentials" => {
            matches!(property, "get" | "create" | "store" | "preventSilentAccess")
        }
        "navigator.permissions" => matches!(property, "query"),
        "navigator.storage" => matches!(
            property,
            "estimate" | "persist" | "persisted" | "getDirectory"
        ),
        "navigator.userAgentData" => matches!(property, "getHighEntropyValues" | "toJSON"),
        _ => false,
    }
}
