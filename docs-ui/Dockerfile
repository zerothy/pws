FROM node:lts-iron as prod
RUN npm install -g pnpm
WORKDIR /app
COPY . .
RUN pnpm install
RUN pnpm build

FROM prod as serve
CMD ["pnpm", "serve", "--host", "0.0.0.0", "--no-open", "--port", "3001"]
