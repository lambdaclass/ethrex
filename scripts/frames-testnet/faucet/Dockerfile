FROM python:3.12-slim
ENV PYTHONUNBUFFERED=1
# The `raw_transaction` attribute exists only from 0.13 (it was `rawTransaction`
# before), so this floor is load-bearing; the ceiling guards a future rename.
RUN pip install --no-cache-dir "eth-account>=0.13,<0.14" && useradd -m faucet
COPY faucet.py page.html /app/
USER faucet
EXPOSE 8080
# PORT is read at runtime, so the probe has to read it too. Hardcoding 8080 here
# made the container report unhealthy forever on any deployment that moved the
# port, while it went on serving perfectly well.
HEALTHCHECK --interval=30s --timeout=5s CMD python -c \
  "import os,urllib.request;urllib.request.urlopen('http://127.0.0.1:'+os.environ.get('PORT','8080')+'/healthz',timeout=4)"
ENTRYPOINT ["python", "/app/faucet.py"]
