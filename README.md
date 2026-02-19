# TTS-service

HTTP microservice using Axum to generate TTS from an HTTP reqwest.

## Modes
- eSpeak - Local TTS, low quality. Returns WAV audio.
- gTTS - Cloud TTS, medium quality. Returns MP3 audio
- gcloud - Google Cloud TTS, high quality. Returns OPUS audio. **Requires a gCloud API key**
- Polly - Amazon Polly TTS, high quality. Returns OggVorbis audio. **Requires Amazon Polly credentials**
- Gwent - Local high quality TTS proxied to a localhost daemon. Defaults to Ogg output.

## Supported endpoints:
- `GET /tts?text={CONTENT}&lang={VOICE}&mode={MODE}&speaking_rate={SPEAKING_RATE}&max_length={MAX_LENGTH}&preferred_format={PREFERRED_AUDIO_FORMAT}` - Returns the audio generated.
- `GET /voices?mode={MODE}&raw={BOOL}` - Returns the supported voices for the given mode as either a JSON array of strings, or a raw format from the source with the `raw` set to true.
- `GET /modes` - Returns the currently supported modes for TTS as a JSON array of strings.

## Error Codes:
Non-200 responses will return a JSON object with the following keys:

### `code` - int
- `0` - Unknown error
- `1` - Unknown voice
- `2` - Max length exceeded
- `3` - Speaking rate exceeded limits, see the `display` for more information
- `4` - `AUTH_KEY` has been set and the `Authorization` header doesn't match the key.
### `display` - str
A human readable message describing the error

## Environment Variables (default)
- `BIND_ADDR`(`0.0.0.0:3000`) - The address to bind the web server to

- `LOG_LEVEL`(`INFO`) - The lowest log level to output to stdout

- `AUTH_KEY` - If set, this key must be sent in the `Authorization` header of each request

### gTTS Required
- `IPV6_BLOCK` - A block of IPv6 addresses, randomly selected for each gTTS request

### gCloud Required
- `GOOGLE_APPLICATION_CREDENTIALS` - The file path to the gCloud JSON

### Polly Required
- `AWS_REGION` - The AWS region to use

- `AWS_ACCESS_KEY_ID` - The AWS access key ID

- `AWS_SECRET_ACCESS_KEY` - The AWS secret access key

### Gwent Required / Optional
- `GWENT_DAEMON_URL` (`http://127.0.0.1:9000`) - Base URL of the local Gwent daemon.
- `GWENT_CONNECT_TIMEOUT_MS` (`500`) - Connect timeout for daemon requests.
- `GWENT_REQUEST_TIMEOUT_MS` (`10000`) - End-to-end timeout for daemon requests.
- `GWENT_MAX_CONCURRENCY` (`32`) - Maximum in-flight daemon requests.
- `GWENT_HEALTH_PATH` (`/health`) - Health endpoint path.
- `GWENT_VOICES_PATH` (`/voices`) - Voice listing endpoint path.
- `GWENT_TTS_PATH` (`/tts`) - Synthesis endpoint path.

## Gwent Daemon Adapter Contract (v1)
`tts-service` expects a local daemon with the following endpoints:

- `GET {GWENT_HEALTH_PATH}` -> returns HTTP 200 when healthy.
- `GET {GWENT_VOICES_PATH}` -> returns either:
  - `["voice_id_1", "voice_id_2"]`, or
  - `[{"id":"voice_id_1","name":"Display Name"}, ...]`
- `POST {GWENT_TTS_PATH}` JSON body:
  - `text` (string)
  - `voice` (string)
  - `speaking_rate` (number)
  - `format` (`"ogg"` or `"mp3"`)
  - `max_length` (optional number)
  Response should be raw audio bytes and ideally set `Content-Type`.
