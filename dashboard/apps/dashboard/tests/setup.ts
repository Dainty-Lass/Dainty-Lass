// Every test file gets a fresh in-memory database: set before lib/config reads the environment.
process.env.DASHBOARD_DATABASE_FILE = ":memory:";
process.env.DASHBOARD_OWNER_DISCORD_IDS = "100000000000000001";
process.env.DASHBOARD_INGEST_KEY = "k".repeat(40);
