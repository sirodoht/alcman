import uuid
from datetime import datetime
from urllib.request import urlopen

from django.utils.text import slugify
from lxml import etree

from main import models


def fetch(subscription):
    with urlopen(subscription.url) as conn:
        xml_content = conn.read()
    return xml_content


def get_subscription_title(subscription):
    xml_content = fetch(subscription)
    data = parse_xml(xml_content)
    return data["title"]


def parse_xml(xml_content):
    root = etree.fromstring(xml_content)
    blog_title = root.find("channel").find("title").text
    article_list = []
    for child in root.find("channel"):
        if child.tag != "item":
            continue

        title = child.find("title").text
        slug = slugify(title) + "-" + str(uuid.uuid4())[:8]
        pub_date = child.find("pubDate").text
        published_at = datetime.strptime(pub_date, "%a, %d %b %Y %H:%M:%S %z")

        article_list.append(
            {
                "title": title,
                "slug": slug,
                "url": child.find("link").text,
                "published_at": published_at,
                "body": child.find("description").text,
            }
        )
    return {
        "title": blog_title,
        "article_list": article_list,
    }


def save_articles(subscription):
    xml_content = fetch(subscription)
    data = parse_xml(xml_content)

    subscription.title = data["title"]
    subscription.save()

    for item in data["article_list"]:
        models.Article.objects.create(subscription=subscription, **item)


def delete_articles(subscription):
    models.Article.objects.filter(subscription=subscription).delete()
