import { setupServer } from "msw/node";
setupServer().listen({ onUnhandledRequest: "bypass" });
