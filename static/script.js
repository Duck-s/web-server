// --- State ---
let selectedServerId = null;
let playerChart = null;
let uptimeChart = null;
let currentRange = "day";
let lastPingId = 0;

// Local history buffers for charts
let historyLabels = [];
let historyPlayerData = [];
let historyUptimeData = [];

// Cache of servers so we do not refetch just to update highlight
let serversCache = [];

// Will be populated after DOMContentLoaded
let dom = {};

// --- DOM helpers ---
function initDom() {
    dom = {
        body: document.body,
        navAuthBtn: document.getElementById("nav-auth-btn"),
        form: document.getElementById("server-form"),
        list: document.getElementById("server-list-container"),
        detailName: document.getElementById("detail-name"),
        detailHost: document.getElementById("detail-host"),
        actions: document.getElementById("action-buttons"),
        statsGrid: document.getElementById("stats-grid"),
        btnPing: document.getElementById("btn-ping-now"), // hidden
        btnDelete: document.getElementById("btn-delete-server"),
        statStatus: document.getElementById("stat-status"),
        statPlayers: document.getElementById("stat-players"),
        statPings: document.getElementById("stat-pings"),
        chartsWrapper: document.getElementById("charts-wrapper"),
        chartOverlay: document.getElementById("chart-overlay"),
        playerCanvas: document.getElementById("playerChart"),
        uptimeCanvas: document.getElementById("uptimeChart"),
        rangeBtns: document.querySelectorAll(".btn-range"),
    };
}

// --- API helper ---
async function api(endpoint, method = "GET", body = null) {
    const url = `/api${endpoint}`;
    const options = {
        method,
        headers: { "Content-Type": "application/json" },
    };

    if (body) {
        options.body = JSON.stringify(body);
    }

    const res = await fetch(url, options);
    if (!res.ok) {
        throw new Error(`API Error: ${res.status}`);
    }
    return res.json();
}

function resetHistory() {
    lastPingId = 0;
    historyLabels = [];
    historyPlayerData = [];
    historyUptimeData = [];
}

// --- Auth ---
async function checkAuth() {
    try {
        const user = await api("/auth/me");
        if (user && user.isAdmin) {
            enableAdminMode();
        }
    } catch (_) {
        // Not logged in or error, ignore
    }
}

function enableAdminMode() {
    if (!dom.navAuthBtn) return;

    dom.body.classList.add("is-admin");
    dom.navAuthBtn.textContent = "Logout";
    dom.navAuthBtn.href = "/auth/logout";
    dom.navAuthBtn.style.backgroundColor = "var(--color-30)";
    dom.navAuthBtn.style.color = "var(--text-main)";
}

// --- Rendering ---
function renderServerList(servers) {
    if (!dom.list) return;

    serversCache = servers || serversCache;

    const scrollPos = dom.list.scrollTop;
    dom.list.textContent = "";

    if (!serversCache || serversCache.length === 0) {
        dom.list.textContent = "No servers found.";
        return;
    }

    serversCache.forEach((s) => {
        const el = document.createElement("div");
        el.className = `server-item ${selectedServerId == s.id ? "active" : ""}`;

        const info = document.createElement("div");
        info.className = "server-info";

        const name = document.createElement("span");
        name.className = "server-name";
        name.textContent = s.name;

        const ip = document.createElement("span");
        ip.className = "server-ip";
        ip.textContent = `${s.address}`;

        info.appendChild(name);
        info.appendChild(ip);

        const dot = document.createElement("div");
        dot.className = `server-status-dot ${s.last_online ? "status-online" : "status-offline"}`;

        el.appendChild(info);
        el.appendChild(dot);

        el.onclick = async () => {
            selectedServerId = s.id;
            renderServerList(); // just re-render using cache for highlight
            await selectServer(s, { skipListRefresh: true });
        };

        dom.list.appendChild(el);
    });

    dom.list.scrollTop = scrollPos;
}

function updateStats(isOnline, players, pingCount) {
    if (!dom.statStatus || !dom.statPlayers || !dom.statPings) return;

    dom.statStatus.textContent = isOnline ? "ONLINE" : "OFFLINE";
    dom.statStatus.style.color = isOnline ? "var(--color-10)" : "var(--danger)";
    dom.statPlayers.textContent = isOnline ? players : "--";
    dom.statPings.textContent = pingCount;

    if (dom.statsGrid) dom.statsGrid.style.opacity = "1";
    if (dom.chartsWrapper) dom.chartsWrapper.style.opacity = "1";
    if (dom.chartOverlay) dom.chartOverlay.style.display = "none";
}

// --- Main logic ---
async function loadServers() {
    try {
        const servers = await api("/servers");
        renderServerList(servers);

        // Auto select first server on initial load
        if (!selectedServerId && servers.length > 0) {
            selectedServerId = servers[0].id;
            await selectServer(servers[0], { skipListRefresh: true });
            renderServerList(); // reuse cache, no extra /servers call
        }
    } catch (e) {
        console.error("Error loading servers", e);
    }
}

async function selectServer(server, options = {}) {
    if (!server) return;

    selectedServerId = server.id;
    resetHistory();

    if (dom.detailName) dom.detailName.textContent = server.name;
    if (dom.detailHost) dom.detailHost.textContent = `${server.address}`;
    if (dom.actions) {
        dom.actions.style.opacity = "1";
        dom.actions.style.pointerEvents = "auto";
    }

    await refreshDashboard();

    // when a user clicks, we already re-render using cache
    if (!options.skipListRefresh) {
        renderServerList(); // uses serversCache, no new fetch
    }
}

async function refreshDashboard() {
    if (!selectedServerId) return;

    try {
        let query = `?range=${encodeURIComponent(currentRange)}`;
        if (lastPingId) {
            query += `&since_id=${encodeURIComponent(lastPingId)}`;
        }

        const newPings = await api(`/servers/${selectedServerId}/pings${query}`);

        if (!lastPingId) {
            resetHistory();
        }

        if (Array.isArray(newPings) && newPings.length > 0) {
            for (const h of newPings) {
                const label = new Date(h.pinged_at).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                });

                const playersField =
                    h.player_count !== undefined && h.player_count !== null
                        ? h.player_count
                        : h.players_online !== undefined && h.players_online !== null
                            ? h.players_online
                            : h.online
                                ? 1
                                : 0;

                const uptime = h.online ? 1 : 0;

                historyLabels.push(label);
                historyPlayerData.push(playersField);
                historyUptimeData.push(uptime);

                if (typeof h.id === "number" && h.id > lastPingId) {
                    lastPingId = h.id;
                }
            }

            const maxPoints = 500;
            if (historyLabels.length > maxPoints) {
                const excess = historyLabels.length - maxPoints;
                historyLabels.splice(0, excess);
                historyPlayerData.splice(0, excess);
                historyUptimeData.splice(0, excess);
            }
        }

        if (historyLabels.length === 0) {
            updateStats(false, 0, 0);

            if (playerChart) {
                playerChart.data.labels = [];
                playerChart.data.datasets[0].data = [];
                playerChart.update("none");
            }

            if (uptimeChart) {
                uptimeChart.data.labels = [];
                uptimeChart.data.datasets[0].data = [];
                uptimeChart.update("none");
            }
            return;
        }

        const latestIndex = historyLabels.length - 1;
        const isOnline = historyUptimeData[latestIndex] === 1;
        const currentPlayers = historyPlayerData[latestIndex];

        updateStats(isOnline, currentPlayers, historyLabels.length);
        updatePlayerChart(historyLabels, historyPlayerData);
        updateUptimeChart(historyLabels, historyUptimeData);
    } catch (e) {
        console.error("Refresh error", e);
    }
}

// --- Charts ---
function updatePlayerChart(labels, dataPoints) {
    if (!dom.playerCanvas) return;

    if (playerChart) {
        playerChart.data.labels = labels;
        playerChart.data.datasets[0].data = dataPoints;
        playerChart.update("none");
        return;
    }

    const ctx = dom.playerCanvas.getContext("2d");
    const gradient = ctx.createLinearGradient(0, 0, 0, 300);
    gradient.addColorStop(0, "rgba(132, 204, 22, 0.2)");
    gradient.addColorStop(1, "rgba(132, 204, 22, 0.0)");

    playerChart = new Chart(ctx, {
        type: "line",
        data: {
            labels,
            datasets: [
                {
                    label: "Players",
                    data: dataPoints,
                    borderColor: "#84cc16",
                    backgroundColor: gradient,
                    borderWidth: 2,
                    fill: true,
                    tension: 0.3,
                    pointRadius: 0,
                    pointHoverRadius: 4,
                    pointHoverBackgroundColor: "#84cc16",
                },
            ],
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            interaction: { intersect: false, mode: "index" },
            animation: false,
            scales: {
                y: {
                    beginAtZero: true,
                    grid: { color: "#262626" },
                    ticks: {
                        color: "#737373",
                        font: { family: "monospace" },
                    },
                },
                x: { display: false },
            },
            plugins: {
                legend: { display: false },
                tooltip: {
                    backgroundColor: "#171717",
                    titleColor: "#fff",
                    bodyColor: "#ccc",
                    borderColor: "#333",
                    borderWidth: 1,
                    displayColors: false,
                },
            },
        },
    });
}

function updateUptimeChart(labels, dataPoints) {
    if (!dom.uptimeCanvas) return;

    if (uptimeChart) {
        uptimeChart.data.labels = labels;
        uptimeChart.data.datasets[0].data = dataPoints;
        uptimeChart.update("none");
        return;
    }

    const ctx = dom.uptimeCanvas.getContext("2d");
    uptimeChart = new Chart(ctx, {
        type: "line",
        data: {
            labels,
            datasets: [
                {
                    label: "Status",
                    data: dataPoints,
                    borderColor: "#84cc16",
                    backgroundColor: "rgba(132, 204, 22, 0.1)",
                    borderWidth: 2,
                    fill: true,
                    stepped: true,
                    pointRadius: 0,
                    pointHoverRadius: 0,
                },
            ],
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            interaction: { intersect: false, mode: "index" },
            animation: false,
            scales: {
                y: {
                    min: 0,
                    max: 1.2,
                    grid: { color: "#262626" },
                    ticks: {
                        color: "#737373",
                        font: { family: "monospace" },
                        callback: (v) => (v === 0 ? "OFF" : v === 1 ? "ON" : ""),
                    },
                },
                x: {
                    grid: { display: false },
                    ticks: {
                        color: "#737373",
                        maxTicksLimit: 8,
                        font: { family: "monospace" },
                    },
                },
            },
            plugins: {
                legend: { display: false },
                tooltip: {
                    backgroundColor: "#171717",
                    titleColor: "#fff",
                    bodyColor: "#ccc",
                    borderColor: "#333",
                    borderWidth: 1,
                    displayColors: false,
                    callbacks: {
                        label: (c) => (c.parsed.y === 1 ? "Online" : "Offline"),
                    },
                },
            },
        },
    });
}

// --- Ten minute aligned scheduler ---
const TEN_MINUTES_MS = 10 * 60 * 1000;

function scheduleAlignedRefresh() {
    const now = new Date();
    const minutes = now.getMinutes();
    const seconds = now.getSeconds();
    const ms = now.getMilliseconds();

    let minutesToNext = (10 - (minutes % 10)) % 10;
    if (minutesToNext === 0 && (seconds > 0 || ms > 0)) {
        minutesToNext = 10;
    }

    let delay =
        minutesToNext * 60 * 1000 -
        seconds * 1000 -
        ms;

    if (delay < 0) {
        delay += TEN_MINUTES_MS;
    }

    setTimeout(async function tick() {
        try {
            const servers = await api("/servers");
            renderServerList(servers);

            if (selectedServerId) {
                await refreshDashboard();
            }
        } catch (e) {
            console.error("Scheduled refresh error", e);
        }

        setInterval(async () => {
            try {
                const servers = await api("/servers");
                renderServerList(servers);

                if (selectedServerId) {
                    await refreshDashboard();
                }
            } catch (e) {
                console.error("Scheduled refresh error", e);
            }
        }, TEN_MINUTES_MS);
    }, delay);
}

// --- Events and init ---
function setupEventListeners() {
    if (dom.form) {
        dom.form.addEventListener("submit", async (e) => {
            e.preventDefault();
            const formData = new FormData(dom.form);

            try {
                await api("/servers", "POST", {
                    name: formData.get("name"),
                    address: formData.get("address"),
                });
                dom.form.reset();
                await loadServers();
            } catch (err) {
                alert("Error adding server");
            }
        });
    }

    if (dom.btnDelete) {
        dom.btnDelete.addEventListener("click", async () => {
            if (!selectedServerId || !confirm("Delete this server?")) {
                return;
            }

            try {
                await api(`/servers/${selectedServerId}`, "DELETE");
                selectedServerId = null;

                resetHistory();

                if (dom.detailName) dom.detailName.textContent = "Overview";
                if (dom.detailHost) dom.detailHost.textContent = "Select a server";
                if (dom.actions) dom.actions.style.opacity = "0";
                if (dom.statsGrid) dom.statsGrid.style.opacity = "0.5";
                if (dom.chartsWrapper) dom.chartsWrapper.style.opacity = "0.3";
                if (dom.chartOverlay) dom.chartOverlay.style.display = "flex";

                if (playerChart) {
                    playerChart.destroy();
                    playerChart = null;
                }
                if (uptimeChart) {
                    uptimeChart.destroy();
                    uptimeChart = null;
                }

                await loadServers();
            } catch (e) {
                console.error(e);
            }
        });
    }

    if (dom.rangeBtns && dom.rangeBtns.length > 0) {
        dom.rangeBtns.forEach((btn) => {
            btn.addEventListener("click", async () => {
                const range = btn.dataset.range;
                if (!range || range === currentRange) return;

                currentRange = range;

                dom.rangeBtns.forEach((b) => {
                    if (b === btn) b.classList.add("active");
                    else b.classList.remove("active");
                });

                resetHistory();
                await refreshDashboard();
            });
        });
    }

    // Hide manual ping button completely
    if (dom.btnPing) {
        dom.btnPing.style.display = "none";
    }
}

async function init() {
    initDom();
    setupEventListeners();

    await checkAuth();
    await loadServers();        // initial data
    scheduleAlignedRefresh();   // aligned updates every 10 minutes
}

// Run once DOM is ready
window.addEventListener("DOMContentLoaded", () => {
    init().catch((e) => console.error("Init error", e));
});
