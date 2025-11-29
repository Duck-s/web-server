// --- State ---
let selectedServerId = null;
let playerChart = null;
let uptimeChart = null;
let currentRange = 'day';
let lastPingId = 0;

// Local history buffers for charts
let historyLabels = [];
let historyPlayerData = [];
let historyUptimeData = [];

// --- DOM Elements ---
const dom = {
    body: document.body,
    navAuthBtn: document.getElementById('nav-auth-btn'),
    form: document.getElementById('server-form'),
    list: document.getElementById('server-list-container'),
    detailName: document.getElementById('detail-name'),
    detailHost: document.getElementById('detail-host'),
    actions: document.getElementById('action-buttons'),
    statsGrid: document.getElementById('stats-grid'),
    btnPing: document.getElementById('btn-ping-now'),
    btnDelete: document.getElementById('btn-delete-server'),
    statStatus: document.getElementById('stat-status'),
    statPlayers: document.getElementById('stat-players'),
    statPings: document.getElementById('stat-pings'),
    chartsWrapper: document.getElementById('charts-wrapper'),
    chartOverlay: document.getElementById('chart-overlay'),
    playerCanvas: document.getElementById('playerChart'),
    uptimeCanvas: document.getElementById('uptimeChart'),
    rangeBtns: document.querySelectorAll('.btn-range'),
};

// --- API Helper ---
async function api(endpoint, method = 'GET', body = null) {
    // All endpoints are now prefixed with /api in main.rs
    const url = `/api${endpoint}`;

    const options = {
        method,
        headers: { 'Content-Type': 'application/json' },
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

// --- Auth Logic ---
async function checkAuth() {
    try {
        const user = await api('/auth/me');
        if (user && user.isAdmin) {
            enableAdminMode();
        }
    } catch (e) {
        // Not logged in
    }
}

function enableAdminMode() {
    dom.body.classList.add('is-admin');
    dom.navAuthBtn.textContent = 'Logout';
    // Auth routes are now under /auth, not /api/auth
    dom.navAuthBtn.href = '/auth/logout';
    dom.navAuthBtn.style.backgroundColor = 'var(--color-30)';
    dom.navAuthBtn.style.color = 'var(--text-main)';
}

// --- Rendering ---
function renderServerList(servers) {
    const scrollPos = dom.list.scrollTop;
    dom.list.textContent = '';

    if (!servers || servers.length === 0) {
        dom.list.textContent = 'No servers found.';
        return;
    }

    servers.forEach((s) => {
        const el = document.createElement('div');
        el.className = `server-item ${selectedServerId == s.id ? 'active' : ''}`;

        const info = document.createElement('div');
        info.className = 'server-info';

        const name = document.createElement('span');
        name.className = 'server-name';
        name.textContent = s.name;

        const ip = document.createElement('span');
        ip.className = 'server-ip';
        ip.textContent = `${s.address}:${s.port}`;

        info.appendChild(name);
        info.appendChild(ip);

        const dot = document.createElement('div');
        dot.className = `server-status-dot ${s.last_online ? 'status-online' : 'status-offline'}`;

        el.appendChild(info);
        el.appendChild(dot);

        el.onclick = () => selectServer(s);
        dom.list.appendChild(el);
    });

    dom.list.scrollTop = scrollPos;
}

function updateStats(isOnline, players, pingCount) {
    dom.statStatus.textContent = isOnline ? 'ONLINE' : 'OFFLINE';
    dom.statStatus.style.color = isOnline ? 'var(--color-10)' : 'var(--danger)';
    dom.statPlayers.textContent = isOnline ? players : '--';
    dom.statPings.textContent = pingCount;

    dom.statsGrid.style.opacity = '1';
    dom.chartsWrapper.style.opacity = '1';
    dom.chartOverlay.style.display = 'none';
}

// --- Logic ---
async function loadServers() {
    try {
        const servers = await api('/servers');
        renderServerList(servers);
        // Auto select first
        if (!selectedServerId && servers.length > 0) {
            selectServer(servers[0]);
        }
    } catch (e) {
        console.error(e);
    }
}

async function selectServer(server) {
    selectedServerId = server.id;

    // Reset incremental history when switching servers
    lastPingId = 0;
    historyLabels = [];
    historyPlayerData = [];
    historyUptimeData = [];

    dom.detailName.textContent = server.name;
    dom.detailHost.textContent = `${server.address}:${server.port}`;
    dom.actions.style.opacity = '1';
    dom.actions.style.pointerEvents = 'auto';

    await refreshDashboard();

    // Re-render list to update highlight
    const servers = await api('/servers');
    renderServerList(servers);
}

async function refreshDashboard() {
    if (!selectedServerId) {
        return;
    }

    try {
        // Build query for history API: range + optional since_id
        let query = `?range=${encodeURIComponent(currentRange)}`;
        if (lastPingId) {
            query += `&since_id=${encodeURIComponent(lastPingId)}`;
        }

        const newPings = await api(`/servers/${selectedServerId}/pings${query}`);

        // If this is the first load for this server / range, start fresh
        if (!lastPingId) {
            historyLabels = [];
            historyPlayerData = [];
            historyUptimeData = [];
        }

        // Merge new data into local history buffers
        if (Array.isArray(newPings) && newPings.length > 0) {
            for (const h of newPings) {
                const label = new Date(h.pinged_at).toLocaleTimeString([], {
                    hour: '2-digit',
                    minute: '2-digit',
                });

                // Support both players_online and player_count field names
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

                if (typeof h.id === 'number' && h.id > lastPingId) {
                    lastPingId = h.id;
                }
            }

            // Optional: cap history length to avoid unbounded growth
            const maxPoints = 500;
            if (historyLabels.length > maxPoints) {
                const excess = historyLabels.length - maxPoints;
                historyLabels.splice(0, excess);
                historyPlayerData.splice(0, excess);
                historyUptimeData.splice(0, excess);
            }
        }

        // If still no data, clear charts and show offline
        if (historyLabels.length === 0) {
            updateStats(false, 0, 0);

            if (playerChart) {
                playerChart.data.labels = [];
                playerChart.data.datasets[0].data = [];
                playerChart.update('none');
            }
            if (uptimeChart) {
                uptimeChart.data.labels = [];
                uptimeChart.data.datasets[0].data = [];
                uptimeChart.update('none');
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
        console.error('Refresh error', e);
    }
}

// --- Charts ---
function updatePlayerChart(labels, dataPoints) {
    if (playerChart) {
        playerChart.data.labels = labels;
        playerChart.data.datasets[0].data = dataPoints;
        playerChart.update('none');
    } else {
        const ctx = dom.playerCanvas.getContext('2d');
        const gradient = ctx.createLinearGradient(0, 0, 0, 300);
        gradient.addColorStop(0, 'rgba(132, 204, 22, 0.2)');
        gradient.addColorStop(1, 'rgba(132, 204, 22, 0.0)');

        playerChart = new Chart(ctx, {
            type: 'line',
            data: {
                labels,
                datasets: [
                    {
                        label: 'Players',
                        data: dataPoints,
                        borderColor: '#84cc16',
                        backgroundColor: gradient,
                        borderWidth: 2,
                        fill: true,
                        tension: 0.3,
                        pointRadius: 0,
                        pointHoverRadius: 4,
                        pointHoverBackgroundColor: '#84cc16',
                    },
                ],
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                interaction: { intersect: false, mode: 'index' },
                animation: false,
                scales: {
                    y: {
                        beginAtZero: true,
                        grid: { color: '#262626' },
                        ticks: {
                            color: '#737373',
                            font: { family: 'monospace' },
                        },
                    },
                    x: { display: false },
                },
                plugins: {
                    legend: { display: false },
                    tooltip: {
                        backgroundColor: '#171717',
                        titleColor: '#fff',
                        bodyColor: '#ccc',
                        borderColor: '#333',
                        borderWidth: 1,
                        displayColors: false,
                    },
                },
            },
        });
    }
}

function updateUptimeChart(labels, dataPoints) {
    if (uptimeChart) {
        uptimeChart.data.labels = labels;
        uptimeChart.data.datasets[0].data = dataPoints;
        uptimeChart.update('none');
    } else {
        const ctx = dom.uptimeCanvas.getContext('2d');
        uptimeChart = new Chart(ctx, {
            type: 'line',
            data: {
                labels,
                datasets: [
                    {
                        label: 'Status',
                        data: dataPoints,
                        borderColor: '#84cc16',
                        backgroundColor: 'rgba(132, 204, 22, 0.1)',
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
                interaction: { intersect: false, mode: 'index' },
                animation: false,
                scales: {
                    y: {
                        min: 0,
                        max: 1.2,
                        grid: { color: '#262626' },
                        ticks: {
                            color: '#737373',
                            font: { family: 'monospace' },
                            callback: (v) => (v === 0 ? 'OFF' : v === 1 ? 'ON' : ''),
                        },
                    },
                    x: {
                        grid: { display: false },
                        ticks: {
                            color: '#737373',
                            maxTicksLimit: 8,
                            font: { family: 'monospace' },
                        },
                    },
                },
                plugins: {
                    legend: { display: false },
                    tooltip: {
                        backgroundColor: '#171717',
                        titleColor: '#fff',
                        bodyColor: '#ccc',
                        borderColor: '#333',
                        borderWidth: 1,
                        displayColors: false,
                        callbacks: {
                            label: (c) => (c.parsed.y === 1 ? 'Online' : 'Offline'),
                        },
                    },
                },
            },
        });
    }
}

// --- Polling ---
setInterval(async () => {
    try {
        const servers = await api('/servers');
        renderServerList(servers);
    } catch (e) {
        // ignore errors for now
    }
}, 10000);

// 2) Graph refresh aligned to backend ping schedule
const PING_INTERVAL_SEC = 600;     // must match main.rs interval
const GRAPH_OFFSET_MS = 4000;      // wait 4s after the ping

async function alignedGraphTick() {
    if (selectedServerId && currentRange === 'day') {
        await refreshDashboard();
    }
}

(function setupGraphScheduler() {
    // do an initial load right away so you are not staring at nothing
    alignedGraphTick();

    // compute time until the NEXT 10 minute boundary
    const nowSec = Math.floor(Date.now() / 1000);
    const secondsPast = nowSec % PING_INTERVAL_SEC;
    const waitSec = PING_INTERVAL_SEC - secondsPast;
    const firstDelayMs = (waitSec * 1000) + GRAPH_OFFSET_MS;

    setTimeout(async function tick() {
        await alignedGraphTick();

        // then keep doing it every 10 minutes
        setInterval(alignedGraphTick, PING_INTERVAL_SEC * 1000);
    }, firstDelayMs);
})();

// --- Event Listeners ---
if (dom.form) {
    dom.form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const formData = new FormData(dom.form);
        try {
            await api('/servers', 'POST', {
                name: formData.get('name'),
                address: formData.get('address'),
                port: parseInt(formData.get('port'), 10),
            });
            dom.form.reset();
            loadServers();
        } catch (err) {
            alert('Error adding server');
        }
    });
}

if (dom.btnPing) {
    dom.btnPing.addEventListener('click', async () => {
        // extra safety: only allow admins to ping
        if (!document.body.classList.contains('is-admin')) {
            alert('Only admin can trigger manual pings.');
            return;
        }

        if (!selectedServerId) {
            return;
        }

        dom.btnPing.disabled = true;
        dom.btnPing.textContent = 'Pinging...';
        try {
            await api(`/servers/${selectedServerId}/ping`);
            await refreshDashboard();
        } catch (e) {
            console.error(e);
        }
        dom.btnPing.disabled = false;
        dom.btnPing.textContent = 'Ping Now';
    });
}

if (dom.btnDelete) {
    dom.btnDelete.addEventListener('click', async () => {
        if (!selectedServerId || !confirm('Delete this server?')) {
            return;
        }
        try {
            await api(`/servers/${selectedServerId}`, 'DELETE');
            selectedServerId = null;

            // Reset local history and charts
            lastPingId = 0;
            historyLabels = [];
            historyPlayerData = [];
            historyUptimeData = [];

            dom.detailName.textContent = 'Overview';
            dom.detailHost.textContent = 'Select a server';
            dom.actions.style.opacity = '0';
            dom.statsGrid.style.opacity = '0.5';
            dom.chartsWrapper.style.opacity = '0.3';
            dom.chartOverlay.style.display = 'flex';

            if (playerChart) {
                playerChart.destroy();
                playerChart = null;
            }
            if (uptimeChart) {
                uptimeChart.destroy();
                uptimeChart = null;
            }

            loadServers();
        } catch (e) {
            console.error(e);
        }
    });
}

// Range buttons (day / week / month)
if (dom.rangeBtns && dom.rangeBtns.length > 0) {
    dom.rangeBtns.forEach((btn) => {
        btn.addEventListener('click', async () => {
            const range = btn.dataset.range;
            if (!range || range === currentRange) return;

            currentRange = range;

            // Update button UI
            dom.rangeBtns.forEach((b) => {
                if (b === btn) b.classList.add('active');
                else b.classList.remove('active');
            });

            // Reset incremental buffers
            lastPingId = 0;
            historyLabels = [];
            historyPlayerData = [];
            historyUptimeData = [];

            // Load full dataset for the new range
            await refreshDashboard();
        });
    });
}

// Init
checkAuth();
loadServers();
