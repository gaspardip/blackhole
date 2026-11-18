import app from "../src/index"; app.request("/", {}, {AUTH_SERVICE: {authenticate: () => null}});
