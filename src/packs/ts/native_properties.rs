fn protected_global(property: &str) -> bool {
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
    owner_properties(owner).is_some_and(|matches| matches(property))
}
pub(super) fn owner_properties(owner: &str) -> Option<fn(&str) -> bool> {
    Some(match owner {
        "" => protected_global,
        "document" | "Document.prototype" => |property| {
            matches!(
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
            )
        },
        "navigator" | "Navigator.prototype" => |property| {
            matches!(
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
            )
        },
        "FileReader.prototype" => |property| {
            matches!(
                property,
                "abort"
                    | "readAsArrayBuffer"
                    | "readAsBinaryString"
                    | "readAsDataURL"
                    | "readAsText"
            )
        },
        "IntersectionObserver.prototype" => |property| {
            matches!(
                property,
                "observe" | "unobserve" | "disconnect" | "takeRecords"
            )
        },
        "ResizeObserver.prototype" => {
            |property| matches!(property, "observe" | "unobserve" | "disconnect")
        }
        "MutationObserver.prototype" => {
            |property| matches!(property, "observe" | "disconnect" | "takeRecords")
        }
        "AbortController.prototype" => |property| matches!(property, "abort" | "signal"),
        "AbortSignal" => |property| matches!(property, "abort" | "any" | "timeout"),
        "AbortSignal.prototype" => {
            |property| matches!(property, "aborted" | "reason" | "throwIfAborted")
        }
        "Date" | "performance" => |property| matches!(property, "now"),
        "Math" => |property| matches!(property, "random"),
        "crypto" => |property| matches!(property, "randomUUID" | "getRandomValues"),
        "history" => |property| {
            matches!(
                property,
                "pushState" | "replaceState" | "go" | "back" | "forward"
            )
        },
        "Storage.prototype" | "localStorage" | "sessionStorage" => |property| {
            matches!(
                property,
                "getItem" | "setItem" | "removeItem" | "clear" | "key" | "length"
            )
        },
        "navigator.clipboard" => {
            |property| matches!(property, "read" | "readText" | "write" | "writeText")
        }
        "navigator.credentials" => {
            |property| matches!(property, "get" | "create" | "store" | "preventSilentAccess")
        }
        "navigator.permissions" => |property| matches!(property, "query"),
        "navigator.storage" => |property| {
            matches!(
                property,
                "estimate" | "persist" | "persisted" | "getDirectory"
            )
        },
        "navigator.userAgentData" => {
            |property| matches!(property, "getHighEntropyValues" | "toJSON")
        }
        _ => return None,
    })
}
