// --- State ---
let selectedServerId = null;
let playerChart = null;
let uptimeChart = null;

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
    uptimeCanvas: document.getElementById('uptimeChart')
};

// --- API Helper ---
async function api(endpoint, method = 'GET', body = null) {
    // All endpoints are now prefixed with /api in main.rs
    const url = `/api${endpoint}`;

    const options = { method, headers: { 'Content-Type': 'application/json' } };
    if (body) options.body = JSON.stringify(body);

    const res = await fetch(url, options);
    if (!res.ok) throw new Error(`API Error: ${res.status}`);
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
    dom.navAuthBtn.textContent = "Logout";
    // Auth routes are now under /auth, not /api/auth
    dom.navAuthBtn.href = "/auth/logout";
    dom.navAuthBtn.style.backgroundColor = "var(--color-30)";
    dom.navAuthBtn.style.color = "var(--text-main)";
}

// --- Rendering ---
function renderServerList(servers) {
    const scrollPos = dom.list.scrollTop;
    dom.list.textContent = '';

    if (!servers || servers.length === 0) {
        dom.list.textContent = 'No servers found.';
        return;
    }

    servers.forEach(s => {
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
    dom.statStatus.textContent = isOnline ? "ONLINE" : "OFFLINE";
    dom.statStatus.style.color = isOnline ? "var(--color-10)" : "var(--danger)";
    dom.statPlayers.textContent = isOnline ? players : "--";
    dom.statPings.textContent = pingCount;

    dom.statsGrid.style.opacity = "1";
    dom.chartsWrapper.style.opacity = "1";
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
    } catch (e) { console.error(e); }
}

async function selectServer(server) {
    selectedServerId = server.id;
    dom.detailName.textContent = server.name;
    dom.detailHost.textContent = `${server.address}:${server.port}`;
    dom.actions.style.opacity = "1";
    dom.actions.style.pointerEvents = "auto";

    await refreshDashboard();

    // Re-render list to update highlight
    const servers = await api('/servers');
    renderServerList(servers);
}

async function refreshDashboard() {
    if (!selectedServerId) return;

    try {
        const history = await api(`/servers/${selectedServerId}/pings`);

        const labels = history.map(h => new Date(h.pinged_at).toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'}));
        const playerData = history.map(h => h.player_count !== undefined ? h.player_count : (h.online ? 1 : 0));
        const uptimeData = history.map(h => h.online ? 1 : 0);

        const latest = history[history.length - 1];
        const isOnline = latest ? latest.online : false;
        const currentPlayers = latest && latest.player_count !== undefined ? latest.player_count : 0;

        updateStats(isOnline, currentPlayers, history.length);
        updatePlayerChart(labels, playerData);
        updateUptimeChart(labels, uptimeData);
    } catch (e) { console.error("Refresh error", e); }
}

// --- Charts ---
function updatePlayerChart(labels, dataPoints) {
    if (playerChart) {
        playerChart.data.labels = labels;
        playerChart.data.datasets[0].data = dataPoints;
        playerChart.update('none');
    } else {
        const ctx = dom.playerCanvas.getContext('2d');
        let gradient = ctx.createLinearGradient(0, 0, 0, 300);
        gradient.addColorStop(0, 'rgba(132, 204, 22, 0.2)');
        gradient.addColorStop(1, 'rgba(132, 204, 22, 0.0)');

        playerChart = new Chart(ctx, {
            type: 'line',
            data: {
                labels: labels,
                datasets: [{
                    label: 'Players',
                    data: dataPoints,
                    borderColor: '#84cc16',
                    backgroundColor: gradient,
                    borderWidth: 2,
                    fill: true,
                    tension: 0.3,
                    pointRadius: 0,
                    pointHoverRadius: 4,
                    pointHoverBackgroundColor: '#84cc16'
                }]
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                interaction: { intersect: false, mode: 'index' },
                animation: false,
                scales: {
                    y: { beginAtZero: true, grid: { color: '#262626' }, ticks: { color: '#737373', font: {family: 'monospace'} } },
                    x: { display: false }
                },
                plugins: { legend: { display: false }, tooltip: { backgroundColor: '#171717', titleColor: '#fff', bodyColor: '#ccc', borderColor: '#333', borderWidth: 1, displayColors: false } }
            }
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
                labels: labels,
                datasets: [{
                    label: 'Status',
                    data: dataPoints,
                    borderColor: '#84cc16',
                    backgroundColor: 'rgba(132, 204, 22, 0.1)',
                    borderWidth: 2,
                    fill: true,
                    stepped: true,
                    pointRadius: 0,
                    pointHoverRadius: 0
                }]
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                interaction: { intersect: false, mode: 'index' },
                animation: false,
                scales: {
                    y: {
                        min: 0, max: 1.2, grid: { color: '#262626' },
                        ticks: { color: '#737373', font: {family: 'monospace'}, callback: v => v === 0 ? 'OFF' : v === 1 ? 'ON' : '' }
                    },
                    x: { grid: { display: false }, ticks: { color: '#737373', maxTicksLimit: 8, font: {family: 'monospace'} } }
                },
                plugins: { legend: { display: false }, tooltip: { backgroundColor: '#171717', titleColor: '#fff', bodyColor: '#ccc', borderColor: '#333', borderWidth: 1, displayColors: false, callbacks: { label: c => c.parsed.y === 1 ? "Online" : "Offline" } } }
            }
        });
    }
}

// --- Polling ---
setInterval(async () => {
    try {
        const servers = await api('/servers');
        renderServerList(servers);
    } catch(e) {}

    if (selectedServerId) {
        await refreshDashboard();
    }
}, 10000); // 10 Seconds

// --- Event Listeners ---
if (dom.form) {
    dom.form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const formData = new FormData(dom.form);
        try {
            await api('/servers', 'POST', {
                name: formData.get('name'),
                address: formData.get('address'),
                port: parseInt(formData.get('port'))
            });
            dom.form.reset();
            loadServers();
        } catch(err) { alert("Error adding server"); }
    });
}

if (dom.btnPing) {
    dom.btnPing.addEventListener('click', async () => {
        if(!selectedServerId) return;
        dom.btnPing.disabled = true;
        dom.btnPing.textContent = "Pinging...";
        try {
            await api(`/servers/${selectedServerId}/ping`);
            await refreshDashboard();
        } catch(e) { console.error(e); }
        dom.btnPing.disabled = false;
        dom.btnPing.textContent = "Ping Now";
    });
}

if (dom.btnDelete) {
    dom.btnDelete.addEventListener('click', async () => {
        if(!selectedServerId || !confirm("Delete this server?")) return;
        try {
            await api(`/servers/${selectedServerId}`, 'DELETE');
            selectedServerId = null;
            dom.detailName.textContent = "Overview";
            dom.detailHost.textContent = "Select a server";
            dom.actions.style.opacity = "0";
            dom.statsGrid.style.opacity = "0.5";
            dom.chartsWrapper.style.opacity = "0.3";
            dom.chartOverlay.style.display = "flex";

            if(playerChart) { playerChart.destroy(); playerChart = null; }
            if(uptimeChart) { uptimeChart.destroy(); uptimeChart = null; }

            loadServers();
        } catch(e) { console.error(e); }
    });
}

// Init
checkAuth();
loadServers();