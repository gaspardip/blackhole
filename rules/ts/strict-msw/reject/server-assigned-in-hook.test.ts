import { setupServer } from "msw/node";
let server; beforeAll(() => { server = setupServer(); server.listen(); });
