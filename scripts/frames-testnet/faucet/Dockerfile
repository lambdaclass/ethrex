FROM python:3.12-slim
ENV PYTHONUNBUFFERED=1
# The `raw_transaction` attribute exists only from 0.13 (it was `rawTransaction`
# before), so this floor is load-bearing; the ceiling guards a future rename.
RUN pip install --no-cache-dir "eth-account>=0.13,<0.14" && useradd -m faucet
COPY faucet.py page.html /app/
USER faucet
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD python -c \
  "import urllib.request;urllib.request.urlopen('http://127.0.0.1:8080/healthz',timeout=4)"
ENTRYPOINT ["python", "/app/faucet.py"]
