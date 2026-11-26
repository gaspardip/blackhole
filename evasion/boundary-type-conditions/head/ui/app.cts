type Example = import("pkg/reverse", {with: {"resolution-mode": "import"}}).Example;
type Other = import("pkg", {with: {"resolution-mode": "import"}}).Example;
import "pkg";
