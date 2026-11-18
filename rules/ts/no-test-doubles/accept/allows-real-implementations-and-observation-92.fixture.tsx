import { vi } from "vitest"; const key = "spyOn"; vi[key](navigator.clipboard, "readText");
