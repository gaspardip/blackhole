type T = import("restricted").Y;
type U = typeof import("restricted");
type V = import("restricted", { with: { "resolution-mode": "import" } }).Y;
