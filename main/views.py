from django.contrib.auth.mixins import LoginRequiredMixin
from django.http import HttpResponseRedirect
from django.shortcuts import render
from django.urls import reverse_lazy
from django.views.generic.edit import CreateView

from main import models


def index(request):
    return render(request, "main/index.html")


class SubscriptionCreateView(LoginRequiredMixin, CreateView):
    model = models.Subscription
    fields = ["title", "url"]
    success_url = reverse_lazy("index")

    def form_valid(self, form):
        self.object = form.save(commit=False)
        self.object.user = self.request.user
        self.object.save()
        return HttpResponseRedirect(self.get_success_url())
