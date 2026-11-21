import { http, HttpResponse } from "msw"; http.get("/api", () => HttpResponse.json({ ok: true }));
