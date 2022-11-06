#!/usr/bin/env bash

set -o errexit
set -o nounset
set -o pipefail
if [[ "${TRACE-0}" == "1" ]]; then
    set -o xtrace
fi

if [[ "${1-}" =~ ^-*h(elp)?$ ]]; then
    echo 'Usage: ./deploy.sh

This script deploys the service in the production server.'
    exit
fi

cd "$(dirname "$0")"

main() {
    # check if venv is enabled
    if [[ -z "${VIRTUAL_ENV}" ]]; then
        exit
    fi

    # make sure linting checks pass
    make lint

    # static
    python manage.py collectstatic --noinput

    # make sure tests pass
    python manage.py test

    # push origin
    git push origin master
    git push github master

    # pull and reload on server
    ssh root@95.217.223.96 'cd /opt/apps/alcman \
        && git pull \
        && source venv/bin/activate \
        && pip install -r requirements.txt \
        && python manage.py collectstatic --noinput \
        && python manage.py migrate \
        && touch /etc/uwsgi/vassals/alcman.ini'
}

main "$@"
