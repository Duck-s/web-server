# HODR 
Simple Minecraft Server Status Dashboard

This is a small project I built to have something clean and useful to host on my domain. It checks Minecraft servers, shows if they’re online, displays player counts, and graphs a bit of history.

## What it does
- Add and track Minecraft servers  
- Shows online/offline status  
- Shows current player count  
- Keeps ping history with charts  
- Basic admin login for managing servers  

## How it’s structured
**Rust backend (Axum) + SQLite + static HTML/CSS/JS frontend.**

## How to run

Set your environment variables:

```bash
$DATABASE_URL=sqlite://sqlite.db
$ADMIN_PASSWORD=youradminpassword
$APP_ENV=production
```
Then start the server:

```bash
cargo run
```

Open in your browser:
```bash
http://localhost:3000
```

## Why I made it

I wanted a fast and simple way to check the status of my Minecraft servers and my friends’ servers without logging in, plus something clean to host on my domain.
