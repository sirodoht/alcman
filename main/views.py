import uuid

from django.contrib.auth.mixins import LoginRequiredMixin
from django.http import HttpResponseRedirect
from django.shortcuts import get_object_or_404, render
from django.urls import reverse_lazy
from django.utils.text import slugify
from django.views.generic.edit import CreateView

from main import models, process_rss


def index(request):
    return render(
        request,
        "main/index.html",
        {
            "subscription_list": models.Subscription.objects.all(),
            "article_list": models.Article.objects.all(),
        },
    )


def subscription(request, subscription_slug):
    return render(
        request,
        "main/index.html",
        {
            "subscription": models.Subscription.objects.get(slug=subscription_slug),
            "subscription_list": models.Subscription.objects.all(),
            "article_list": models.Article.objects.filter(
                subscription__slug=subscription_slug
            ),
        },
    )


def article(request, article_slug):
    article = get_object_or_404(models.Article, slug=article_slug)
    return render(
        request,
        "main/article.html",
        {
            "article": article,
            "subscription_list": models.Subscription.objects.all(),
        },
    )


class SubscriptionCreateView(LoginRequiredMixin, CreateView):
    model = models.Subscription
    fields = ["url"]
    success_url = reverse_lazy("index")

    def form_valid(self, form):
        self.object = form.save(commit=False)
        self.object.user = self.request.user
        self.object.title = process_rss.get_subscription_title(self.object)
        self.object.slug = slugify(self.object.title) + "-" + str(uuid.uuid4())[:8]
        self.object.save()
        return HttpResponseRedirect(self.get_success_url())

    def get_context_data(self, **kwargs):
        context = super().get_context_data(**kwargs)
        context["subscription_list"] = models.Subscription.objects.all()
        return context
