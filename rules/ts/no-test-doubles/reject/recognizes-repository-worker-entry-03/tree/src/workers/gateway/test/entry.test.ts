import app from "../src/index.ts"; app.request("/", {}, {AUTH_SERVICE: {authenticate: () => null}});
