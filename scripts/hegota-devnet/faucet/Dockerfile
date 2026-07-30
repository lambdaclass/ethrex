FROM python:3.12-slim
RUN pip install --no-cache-dir "eth-account>=0.13" && useradd -m faucet
COPY faucet.py /app/faucet.py
USER faucet
EXPOSE 8080
ENTRYPOINT ["python", "/app/faucet.py"]
