const { Eyes, Target, Region, Configuration, BatchInfo } = require('@applitools/eyes-images');
const fs = require('fs');

async function run() {
    const args = process.argv.slice(2);
    if (args.length < 5) {
        console.error("Usage: node applitools_bridge.js <apiKey> <appName> <testName> <originalImg> <modifiedImg> [ignoreRegionsJson]");
        process.exit(1);
    }

    const apiKey = args[0];
    const appName = args[1];
    const testName = args[2];
    const originalImg = args[3];
    const modifiedImg = args[4];
    const ignoreRegionsStr = args.length > 5 ? args[5] : "[]";

    let ignoreRegions = [];
    try {
        ignoreRegions = JSON.parse(ignoreRegionsStr);
    } catch (e) {
        console.error("Failed to parse ignoreRegions:", e);
    }

    if (!fs.existsSync(originalImg) || !fs.existsSync(modifiedImg)) {
        console.error("Image files do not exist");
        process.exit(1);
    }

    const batch = new BatchInfo("Bank Statement Validation");

    // Pass 1: Establish the Baseline
    let eyes1 = new Eyes();
    eyes1.setApiKey(apiKey);
    let conf1 = new Configuration();
    conf1.setBatch(batch);
    eyes1.setConfiguration(conf1);

    try {
        await eyes1.open(appName, testName);
        await eyes1.check('Page Baseline', Target.image(originalImg));
        await eyes1.close(false); // Do not throw if it's a new baseline
    } catch (e) {
        console.error("Applitools baseline upload failed:", e);
        if (eyes1.getIsOpen()) {
            await eyes1.abort();
        }
        process.exit(1);
    }

    // Pass 2: Verify the Checkpoint
    let eyes2 = new Eyes();
    eyes2.setApiKey(apiKey);
    let conf2 = new Configuration();
    conf2.setBatch(batch);
    eyes2.setConfiguration(conf2);

    let resultJson = { passed: false, url: null, error: null };
    try {
        await eyes2.open(appName, testName);
        let target = Target.image(modifiedImg);
        
        for (let r of ignoreRegions) {
            target = target.ignoreRegion(new Region(r.left, r.top, r.width, r.height));
        }

        await eyes2.check('Page Checkpoint', target);
        let result = await eyes2.close(false);

        resultJson.passed = result.isPassed();
        resultJson.url = result.getUrl();
    } catch (e) {
        resultJson.error = e.toString();
        if (eyes2.getIsOpen()) {
            await eyes2.abort();
        }
    }

    console.log("APPLITOOLS_RESULT:" + JSON.stringify(resultJson));
}

run();
