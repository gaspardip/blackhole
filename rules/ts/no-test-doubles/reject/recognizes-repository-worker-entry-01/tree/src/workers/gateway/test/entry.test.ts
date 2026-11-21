import app from "../src"; app.request("/", {}, {AUTH_SERVICE: {authenticate: () => null}});
