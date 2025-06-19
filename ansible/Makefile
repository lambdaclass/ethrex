.PHONY: all

all: inventory ethrex-l2

inventory:
	@if [ -z "$(DATABASE_IP)" ] || [ -z "$(L1_IP)" ] || [ -z "$(L2_IP)" ] || [ -z "$(MONITORING_IP)" ] || [ -z "$(EXPLORER_IP)" ]; then echo "Error: All variables (ETHREX_L1, ETHREX_L2, DATABASE, MONITORING, EXPLORER) must be set.\n\n  Usage: make inventory ETHREX_L1='ip1,ip2,...' ETHREX_L2='ip1,ip2,...' DATABASE='ip1,ip2,...' MONITORING='ip1,ip2,...' EXPLORER='ip1,ip2,...' \n" 1>&2; exit 1; fi; \
	truncate -s0 inventory.ini
	echo "[database]" >> inventory.ini; \
	for ip in $$(echo $(DATABASE_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
	echo "" >> inventory.ini; \
	echo "[explorer]" >> inventory.ini; \
	for ip in $$(echo $(EXPLORER_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done
	echo "" >> inventory.ini; \
	echo "[l1]" >> inventory.ini; \
	for ip in $$(echo $(L1_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
	echo "" >> inventory.ini; \
	echo "[l2]" >> inventory.ini; \
	for ip in $$(echo $(L2_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> inventory.ini; done; \
	echo "" >> inventory.ini; \
	echo "[metrics]" >> inventory.ini; \
	for ip in $$(echo $(METRICS_IP) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args='-o ForwardAgent=yes'" >> inventory.ini; done; \
	

database:
	ansible-playbook -i inventory.ini database.yml

ethrex-l1:
	ANSIBLE_ENVVAR=$(ENVVAR) ansible-playbook -i inventory.ini ethrex-l1.yml

ethrex-l2:
	ansible-playbook -i inventory.ini ethrex-l2.yml

explorer_backend:
	ansible-playbook -i inventory.ini explorer_backend.yml

explorer_fontend:
	ansible-playbook -i inventory.ini explorer_fontend.yml

metrics:
	ansible-playbook -i inventory.ini metrics.yml
