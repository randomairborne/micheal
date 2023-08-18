# micheal

a bot to record discord audio and POST it to a handling endpoint. micheal is available on ghcr:

```bash
docker run ghcr.io/randomairborne/micheal
```

```dotenv
DISCORD_TOKEN=(your token)
GUILD_ID=(guild id you want it to join)
CHANNEL_ID=(channel id you want it to join)
ENDPOINT_TOKEN=(token to be sent in the Authorization header as a Bearer)
ENDPOINT=(where to send the audio waveform)
RUST_LOG=micheal=info
```
