const positions = [
  { name: "Delta Forge", size: "$1.8M", ltv: 0.61, health: 0.82, maturity: "21d", status: "open", yield: "7.4%", chain: "Testnet" },
  { name: "Orion Metals", size: "$930k", ltv: 0.78, health: 0.56, maturity: "9d", status: "warning", yield: "8.1%", chain: "Testnet" },
  { name: "Tidal Labs", size: "$420k", ltv: 0.49, health: 0.92, maturity: "44d", status: "open", yield: "6.3%", chain: "Localnet" },
  { name: "Atlas Finance", size: "$3.1M", ltv: 0.69, health: 0.35, maturity: "3d", status: "liquidation", yield: "10.2%", chain: "Testnet" },
  { name: "Quartz Rail", size: "$760k", ltv: 0.58, health: 0.73, maturity: "18d", status: "open", yield: "6.8%", chain: "Localnet" },
  { name: "Nova Harbor", size: "$2.4M", ltv: 0.65, health: 0.84, maturity: "30d", status: "settled", yield: "7.9%", chain: "Testnet" },
];

const oracles = [
  { asset: "SOL/USD", price: "$143.22", conf: "0.15%", status: "good" },
  { asset: "ETH/USD", price: "$3,450.12", conf: "0.12%", status: "good" },
  { asset: "BTC/USD", price: "$65,880.10", conf: "0.18%", status: "stale" },
  { asset: "XRS/USDC", price: "$1.03", conf: "0.24%", status: "stale" },
  { asset: "USDC/USD", price: "$1.00", conf: "0.02%", status: "good" },
];

const timelineEvents = [
  { label: "Open margin account", eta: "Now", accent: "success" },
  { label: "TWAP snapshot", eta: "02:14", accent: "neutral" },
  { label: "Maturity check", eta: "05:40", accent: "warning" },
  { label: "Auto-liquidation window", eta: "08:10", accent: "danger" },
];

const queue = [
  { title: "Rebalance LTV", desc: "Decrease LTV on Orion Metals by 4%", badge: "warning" },
  { title: "Trigger settlement", desc: "Finalize Nova Harbor repayment", badge: "success" },
  { title: "Refresh oracle", desc: "Force fresh XRS/USDC snapshot", badge: "neutral" },
];

const statusColors = {
  open: "success",
  warning: "warning",
  liquidation: "danger",
  settled: "neutral",
};

function renderPositions(filter = "all") {
  const container = document.getElementById("positionCards");
  container.innerHTML = "";
  positions
    .filter((p) => filter === "all" || p.status === filter)
    .forEach((p) => {
      const card = document.createElement("div");
      card.className = "card";
      card.innerHTML = `
        <div class="card-header">
          <div>
            <div class="card-title">${p.name}</div>
            <div class="card-meta">${p.chain} · ${p.maturity} maturity</div>
          </div>
          <span class="badge ${statusColors[p.status]}">${p.status}</span>
        </div>
        <div class="row">
          <span class="label">Exposure</span>
          <span class="value">${p.size}</span>
        </div>
        <div class="row">
          <span class="label">LTV</span>
          <span class="value">${Math.round(p.ltv * 100)}%</span>
        </div>
        <div class="bar"><span style="width:${Math.min(p.ltv * 100, 100)}%"></span></div>
        <div class="row">
          <span class="label">Health</span>
          <span class="value">${Math.round(p.health * 100)}%</span>
        </div>
        <div class="bar"><span style="width:${Math.min(p.health * 100, 100)}%; background: linear-gradient(90deg, #7ef3b5, #7ae3ff);"></span></div>
        <div class="row">
          <span class="label">APY</span>
          <span class="value">${p.yield}</span>
        </div>
      `;
      container.appendChild(card);
    });
}

function renderOracles(showStale = true) {
  const container = document.getElementById("oracleList");
  container.innerHTML = "";
  oracles
    .filter((o) => showStale || o.status !== "stale")
    .forEach((o) => {
      const oracle = document.createElement("div");
      oracle.className = "oracle";
      oracle.innerHTML = `
        <div>
          <div class="card-title">${o.asset}</div>
          <div class="card-meta">Confidence ${o.conf}</div>
        </div>
        <div class="value">${o.price}</div>
        <div class="status">
          <span class="status-dot ${o.status}"></span>
          <span class="badge ${statusColors[o.status] || "neutral"}">${o.status}</span>
        </div>
      `;
      container.appendChild(oracle);
    });
}

function renderTimeline() {
  const container = document.getElementById("timeline");
  container.innerHTML = "";
  timelineEvents.forEach((evt) => {
    const item = document.createElement("div");
    item.className = "timeline-item";
    item.innerHTML = `
      <div class="label">${evt.label}</div>
      <div class="chip ${evt.accent}">${evt.eta}</div>
    `;
    container.appendChild(item);
  });
}

function renderHealth() {
  const grid = document.getElementById("healthGrid");
  grid.innerHTML = "";
  const snapshots = [
    { label: "Financing", health: 0.89, desc: "LTV within guardrails" },
    { label: "Liquidation", health: 0.71, desc: "2 positions on watch" },
    { label: "Oracle", health: 0.94, desc: "Feeds fresh & verified" },
    { label: "LP Vault", health: 0.82, desc: "USDC utilization 61%" },
    { label: "Settlement", health: 0.77, desc: "3 maturities today" },
  ];
  snapshots.forEach((snap) => {
    const card = document.createElement("div");
    card.className = "health-card";
    card.innerHTML = `
      <div class="title">${snap.label}</div>
      <div class="metric">${Math.round(snap.health * 100)}%</div>
      <div class="bar"><span style="width:${snap.health * 100}%; background: linear-gradient(90deg, #a1ff6c, #7ae3ff);"></span></div>
      <div class="sub">${snap.desc}</div>
    `;
    grid.appendChild(card);
  });
}

function renderQueue() {
  const container = document.getElementById("actionQueue");
  container.innerHTML = "";
  queue.forEach((item) => {
    const row = document.createElement("div");
    row.className = "queue-item";
    row.innerHTML = `
      <div>
        <div class="card-title">${item.title}</div>
        <div class="card-meta">${item.desc}</div>
      </div>
      <span class="badge ${item.badge}">${item.badge}</span>
    `;
    container.appendChild(row);
  });
}

function wireInteractions() {
  document.getElementById("statusFilter").addEventListener("change", (e) => {
    renderPositions(e.target.value);
  });

  document.getElementById("refresh").addEventListener("click", () => {
    const hero = document.querySelector(".hero");
    hero.classList.add("pulse");
    setTimeout(() => hero.classList.remove("pulse"), 500);
  });

  let showStale = true;
  document.getElementById("toggleStale").addEventListener("click", () => {
    showStale = !showStale;
    renderOracles(showStale);
  });

  document.getElementById("clearQueue").addEventListener("click", () => {
    queue.length = 0;
    renderQueue();
  });
}

function init() {
  renderPositions();
  renderOracles();
  renderTimeline();
  renderHealth();
  renderQueue();
  wireInteractions();
}

init();
