import base from "./playwright.config.mjs";

// npm run shots: e2e/shots.spec.js skips unless SHOTS is 1, and the workers inherit this process's environment.
process.env.SHOTS = "1";

export default { ...base, testMatch: "shots.spec.js" };
