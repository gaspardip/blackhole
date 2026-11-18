import Knock from "@knocklabs/node"; const options = { fetch: () => Response.json({}) }; new Knock(options); new Knock({ ...options });
