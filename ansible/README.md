# the ansible stuff

## example `ansible/inventory.yml`

You have to create an `ansible/inventory.yml` before you can run the playbook

```yml
---
local:
  hosts:
    localhost:
      ansible_connection: local
      pipo_user:                                                     # required, must be "pipo_user"
        from:
          user: example_user                                         # required, your current username (needs sudo)
        to:
          user: pipo                                                 # required, must be "pipo"

server_pipo:
  hosts:
    "pipo.example.com":
      pipo_registration_file: /home/pipo/pipo/registration_pipo.yaml # required, path to the registration file pipo generates
      pipo_repository: https://github.com/Sife-ops/pipo              # optional
      pipo_version: "feature/ansible"                                # optional
      
server_synapse:
  hosts:
    "matrix.example.com":
      pipo_appservice: "pipo.example.com"                            # required, host to reference as the pipo appservice
      synapse_data_path: /root/synapse_matrix.example.com            # required, path to synapse data (containing `homeserver.yaml`)
      synapse_container_name: synapse_matrix.example.com             # optional, the name of the container to be restarted
```

## run

Run the first playbook to set up repo/dependencies

```bash
./ansible/00.yml
```

Run the next playbook after the registration generates

```bash
./ansible/01.yml
```

## note

You should install rustup by yourself when you login as the user to build pipo

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
