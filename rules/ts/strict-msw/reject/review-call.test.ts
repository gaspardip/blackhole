import { setupServer } from "msw/node"; const server = setupServer(); server.listen.call(server, {onUnhandledRequest: "bypass"});
