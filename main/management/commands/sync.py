from django.core.management.base import BaseCommand
from django.utils import timezone

from main import models, process_rss


class Command(BaseCommand):
    help = "Sync all current subscriptions."

    def add_arguments(self, parser):
        parser.add_argument("--no-create", action="store_true")

    def handle(self, *args, **options):
        subscription_list = models.Subscription.objects.all()
        for subscription in subscription_list:
            subscription.user.sync_ongoing = True
            subscription.user.synced_at = timezone.now()
            subscription.user.save()
            self.stdout.write(self.style.SUCCESS(f"Now processing {subscription.url}."))

            # delete all existing ones
            process_rss.delete_articles(subscription)

            # fetch new
            if not options["no_create"]:
                process_rss.save_articles(subscription)

            subscription.user.sync_ongoing = False
            subscription.user.synced_at = timezone.now()
            subscription.user.save()

        self.stdout.write(self.style.SUCCESS("All subscriptions processed."))
