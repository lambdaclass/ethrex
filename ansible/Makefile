.PHONY: all

all: inventory ethrex-l2

inventory:
	@if [ -z "$(TARGET)" ]; then echo "Error: TARGET variable is not set. Usage: make inventory TARGET='ip1,ip2, ...'" 1>&2; exit 1; fi; \
		echo "[all]" > ansible/inventory.yml; \
		for ip in $$(echo $(TARGET) | tr ',' ' '); do echo "$$ip ansible_ssh_user=admin ansible_python_interpreter=/usr/bin/python3 ansible_ssh_common_args='-o ForwardAgent=yes'" >> ansible/inventory.yml; done

ethrex-l2:
	ansible-playbook -i inventory.yml ethrex-l2.yml

explorer:
	ansible-playbook -i inventory.yml explorer.yml
