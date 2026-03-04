FROM nginx:alpine
COPY dashboard.html /usr/share/nginx/html/index.html
COPY index.html /usr/share/nginx/html/bridge.html
COPY withdraw-status.html /usr/share/nginx/html/withdraw-status.html
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
EXPOSE 80
ENTRYPOINT ["/entrypoint.sh"]
