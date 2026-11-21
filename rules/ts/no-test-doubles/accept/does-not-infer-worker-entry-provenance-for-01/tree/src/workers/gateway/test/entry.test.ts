import app from "../client"; app.request("/", {}, {AUTH_SERVICE: {authenticate: () => null}});
