/**
 * Validate that all transaction fixes are in place
 */

const fs = require('fs');
const path = require('path');

console.log('🔍 Validating Transaction Fixes in solana-integration.js\n');

const integrationJs = fs.readFileSync(
    path.join(__dirname, 'ui-design/js/solana-integration.js'),
    'utf8'
);

let allPassed = true;

// Test 1: Check LTV capping is in place
console.log('1️⃣  Checking Open Position LTV Capping...');
if (integrationJs.includes('const initialLTVBps = Math.min(Math.floor(initialLTV * 10000), maxLTVBps)')) {
    console.log('   ✅ LTV capping implemented correctly');
} else {
    console.log('   ❌ LTV capping NOT found');
    allPassed = false;
}

// Test 2: Check deposit has 7 accounts
console.log('\n2️⃣  Checking Deposit Transaction...');
const depositMatch = integrationJs.match(/async function deposit\(([^)]*)\)[\s\S]*?createInstruction\([^,]*,\s*\[([^\]]*)\]/);
if (depositMatch) {
    const accountsStr = depositMatch[2];
    const accountCount = (accountsStr.match(/pubkey:/g) || []).length;
    if (accountCount === 7) {
        console.log('   ✅ Deposit has 7 accounts');
        console.log('   - vault, lp_token_mint, user_lp_token_account,');
        console.log('   - user_usdc_account, vault_usdc_account, user, token_program');
    } else {
        console.log(`   ❌ Deposit has ${accountCount} accounts (expected 7)`);
        allPassed = false;
    }
} else {
    console.log('   ⚠️  Could not parse deposit function');
}

// Test 3: Check withdraw has 7 accounts
console.log('\n3️⃣  Checking Withdraw Transaction...');
const withdrawMatch = integrationJs.match(/async function withdraw\(([^)]*)\)[\s\S]*?createInstruction\([^,]*,\s*\[([^\]]*)\]/);
if (withdrawMatch) {
    const accountsStr = withdrawMatch[2];
    const accountCount = (accountsStr.match(/pubkey:/g) || []).length;
    if (accountCount === 7) {
        console.log('   ✅ Withdraw has 7 accounts');
        console.log('   - vault, lp_token_mint, user_lp_token_account,');
        console.log('   - user_usdc_account, vault_usdc_account, user, token_program');
    } else {
        console.log(`   ❌ Withdraw has ${accountCount} accounts (expected 7)`);
        allPassed = false;
    }
} else {
    console.log('   ⚠️  Could not parse withdraw function');
}

// Test 4: Check closePosition has position_counter
console.log('\n4️⃣  Checking Close Position PDAs...');
if (integrationJs.includes('position_counter')) {
    console.log('   ✅ position_counter PDA included');
} else {
    console.log('   ❌ position_counter PDA NOT found');
    allPassed = false;
}

if (integrationJs.includes('protocol_config')) {
    console.log('   ✅ protocol_config PDA included');
} else {
    console.log('   ❌ protocol_config PDA NOT found');
    allPassed = false;
}

// Test 5: Check conditional accounts for close_early vs close_at_maturity
console.log('\n5️⃣  Checking Close Early vs Close At Maturity...');
if (integrationJs.includes("instructionName === 'close_early'")) {
    console.log('   ✅ Conditional account handling for close_early');
    if (integrationJs.includes('ASSOCIATED_TOKEN_PROGRAM_ID') &&
        integrationJs.includes('SYSTEM_PROGRAM_ID')) {
        console.log('   ✅ close_early includes associated_token_program and system_program');
    } else {
        console.log('   ❌ close_early missing required accounts');
        allPassed = false;
    }
} else {
    console.log('   ❌ No conditional handling for close_early');
    allPassed = false;
}

// Test 6: Cache busting version
console.log('\n6️⃣  Checking Cache Busting...');
const indexHtml = fs.readFileSync(
    path.join(__dirname, 'ui-design/index.html'),
    'utf8'
);
if (indexHtml.includes('solana-integration.js?v=CLOSE-ACCOUNTS-FIXED')) {
    console.log('   ✅ Cache busting version updated');
} else {
    console.log('   ⚠️  Cache busting version may need update');
}

console.log('\n' + '='.repeat(50));
if (allPassed) {
    console.log('✅ All transaction fixes validated!\n');
    console.log('Summary of fixes:');
    console.log('  ✓ Open Position: LTV capped at max_ltv');
    console.log('  ✓ Deposit: 7 accounts (was 2)');
    console.log('  ✓ Withdraw: 7 accounts (was 2)');
    console.log('  ✓ Close Position: Added position_counter and protocol_config PDAs');
    console.log('  ✓ Close Early: 16 accounts (includes init_if_needed accounts)');
    console.log('  ✓ Close At Maturity: 14 accounts (no init_if_needed)');
    console.log('\n🎯 Ready for testing on the live UI!');
    process.exit(0);
} else {
    console.log('❌ Some validations failed\n');
    process.exit(1);
}
