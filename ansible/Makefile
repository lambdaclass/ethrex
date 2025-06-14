.PHONY: all

all: inventory ethrex-l2

inventory:
	@if [ -z "$(ETHREX_L1)" ] || [ -z "$(ETHREX_L2)" ] || [ -z "$(DATABASE)" ] || [ -z "$(MONITORING)" ] || [ -z "$(EXPLORER)" ]; then echo "Error: All variables (ETHREX_L1, ETHREX_L2, DATABASE, MONITORING, EXPLORER) must be set.\n\n  Usage: make inventory ETHREX_L1='ip1,ip2,...' ETHREX_L2='ip1,ip2,...' DATABASE='ip1,ip2,...' MONITORING='ip1,ip2,...' EXPLORER='ip1,ip2,...' \n" 1>&2; exit 1; fi; \
	echo "[ethrex_l1]" >> ansible/inventory.ini; \
	for ip in $$(echo $(ETHREX_L1) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> ansible/inventory.ini; done; \
	echo "" >> ansible/inventory.ini; \
	echo "[ethrex_l2]" >> ansible/inventory.ini; \
	for ip in $$(echo $(ETHREX_L2) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> ansible/inventory.ini; done; \
	echo "" >> ansible/inventory.ini; \
	echo "[database]" >> ansible/inventory.ini; \
	for ip in $$(echo $(DATABASE) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> ansible/inventory.ini; done; \
	echo "" >> ansible/inventory.ini; \
	echo "[monitoring]" >> ansible/inventory.ini; \
	for ip in $$(echo $(MONITORING) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args='-o ForwardAgent=yes'" >> ansible/inventory.ini; done; \
	echo "" >> ansible/inventory.ini; \
	echo "[explorer]" >> ansible/inventory.ini; \
	for ip in $$(echo $(EXPLORER) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3" >> ansible/inventory.ini; done

ethrex-l2:
	ansible-playbook -i inventory.ini ethrex-l2.yml

ethrex-l1:
	ansible-playbook -i inventory.ini ethrex-l1.yml

database:
	ansible-playbook -i inventory.ini database.yml

explorer:
	ansible-playbook -i inventory.ini explorer.yml

metrics:
	ansible-playbook -i inventory.ini metrics.yml
