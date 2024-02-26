#!/bin/sh

export PGHOST=postgres
export PGUSER=postgres

createuser synapse_user && \
createdb --encoding=UTF8 --locale=C --template=template0 --owner=synapse_user synapse
