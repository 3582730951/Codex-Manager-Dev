FROM node:22-alpine
WORKDIR /app

RUN apk add --no-cache chromium nss freetype harfbuzz ca-certificates ttf-freefont

COPY services/browser-assist/package.json ./package.json
RUN npm install --omit=dev

COPY services/browser-assist/server.mjs ./server.mjs

ENV PORT=8090
ENV CMGR_BROWSER_ASSIST_CHROMIUM_PATH=/usr/bin/chromium-browser

EXPOSE 8090

CMD ["node", "server.mjs"]
